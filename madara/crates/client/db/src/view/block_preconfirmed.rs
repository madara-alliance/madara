use crate::{
    preconfirmed::{PreconfirmedBlock, PreconfirmedBlockInner, PreconfirmedExecutedTransaction},
    prelude::*, rocksdb::RocksDBStorage,
};
use mp_block::{header::PreconfirmedHeader, MadaraPreconfirmedBlockInfo, PreconfirmedFullBlock, TransactionWithReceipt};
use mp_class::ConvertedClass;
use mp_receipt::EventWithTransactionHash;
use mp_state_update::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateDiff,
    StorageEntry,
};
use mp_transactions::TransactionWithHash;
use std::fmt;

/// Lock guard on the content of a preconfirmed block. Only the first [`n_txs_visible`] executed transactions
/// are visible.
pub(super) struct PreconfirmedContentRef<'a, D: MadaraStorageRead> {
    guard: tokio::sync::watch::Ref<'a, PreconfirmedBlockInner>,
    view: &'a MadaraPreconfirmedBlockView<D>,
}
impl<'a, D: MadaraStorageRead> PreconfirmedContentRef<'a, D> {
    pub fn executed_transactions(
        &self,
    ) -> impl DoubleEndedIterator<Item = &PreconfirmedExecutedTransaction> + Clone + ExactSizeIterator {
        self.guard.executed_transactions().take(self.view.n_txs_visible)
    }
}

/// A view on a preconfirmed block. If the underlying preconfirmed block is changed, the changes will not be visible until you refresh
/// the view.
///
/// Note: The Eq/PartialEq implementation uses Arc::ptr_eq only. Two preconfirmed blocks with the same content
/// and header will appear as different preconfirmed blocks if they do not originate from the same preconfirmed instance
/// (ie. different block proposals).
#[derive(Debug)]
pub struct MadaraPreconfirmedBlockView<D: MadaraStorageRead = RocksDBStorage> {
    backend: Arc<MadaraBackend<D>>,

    /// Number of transactions visible in the block.
    n_txs_visible: usize,
    block: Arc<PreconfirmedBlock>,
    block_content: tokio::sync::watch::Receiver<PreconfirmedBlockInner>,

    /// Candidate transactions. Most of the time, we don't care about those, so this vec is empty.
    /// This vec is only filled when using `reset_with_candidates`.
    candidates: Vec<TransactionWithHash>,
}

// derive(Clone) will put a D: Clone bounds which we don't want, so we have to implement clone by hand :(
impl<D: MadaraStorageRead> Clone for MadaraPreconfirmedBlockView<D> {
    fn clone(&self) -> Self {
        Self {
            backend: self.backend.clone(),
            n_txs_visible: self.n_txs_visible,
            block: self.block.clone(),
            block_content: self.block_content.clone(),
            candidates: self.candidates.clone(),
        }
    }
}

impl<D: MadaraStorageRead> PartialEq for MadaraPreconfirmedBlockView<D> {
    fn eq(&self, other: &Self) -> bool {
        self.n_txs_visible == other.n_txs_visible && Arc::ptr_eq(&self.block, &other.block)
    }
}
impl<D: MadaraStorageRead> Eq for MadaraPreconfirmedBlockView<D> {}

impl<D: MadaraStorageRead> fmt::Display for MadaraPreconfirmedBlockView<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.candidates.is_empty() {
            write!(
                f,
                "[Preconfirmed block at height #{} with {} transactions]",
                self.block.header.block_number,
                self.num_executed_transactions()
            )
        } else {
            write!(
                f,
                "[Preconfirmed block at height #{} with {} transactions and {} candidates]",
                self.block.header.block_number,
                self.num_executed_transactions(),
                self.candidates.len()
            )
        }
    }
}

impl<D: MadaraStorageRead> MadaraPreconfirmedBlockView<D> {
    /// New view on block start (no transaction).
    pub fn new_at_start(backend: Arc<MadaraBackend<D>>, block: Arc<PreconfirmedBlock>) -> Self {
        let mut block_content = block.content.subscribe();
        block_content.mark_changed(); // mark outdated
        Self { backend, n_txs_visible: 0, block_content, block, candidates: vec![] }
    }
    /// New view on the current block state.
    pub fn new(backend: Arc<MadaraBackend<D>>, block: Arc<PreconfirmedBlock>) -> Self {
        let mut this = Self::new_at_start(backend, block);
        this.refresh();
        this
    }

    pub fn backend(&self) -> &Arc<MadaraBackend<D>> {
        &self.backend
    }

    pub fn parent_block(&self) -> Option<MadaraConfirmedBlockView<D>> {
        self.block_number()
            .checked_sub(1)
            .map(|block_number| MadaraConfirmedBlockView::new(self.backend().clone(), block_number))
    }

    pub fn state_view_on_parent(&self) -> MadaraStateView<D> {
        MadaraStateView::on_confirmed_or_empty(self.backend().clone(), self.block_number().checked_sub(1))
    }

    pub fn state_view(&self) -> MadaraStateView<D> {
        self.clone().into()
    }

    /// Returns a lock guard into the current block content.
    pub(super) fn borrow_content(&self) -> PreconfirmedContentRef<'_, D> {
        PreconfirmedContentRef { guard: self.block_content.borrow(), view: self }
    }

    /// If the pre-confirmed block has changed, this will mark the new transactions as visible.
    /// Clears any seen candidate transactions, if any.
    pub fn refresh(&mut self) {
        self.n_txs_visible = self.block_content.borrow_and_update().n_executed();
        self.candidates.clear();
    }

    /// If the pre-confirmed block has changed, this will mark the new transactions as visible.
    /// Candidate transactions will also be visible.
    pub fn refresh_with_candidates(&mut self) {
        let borrow = self.block_content.borrow_and_update();
        self.n_txs_visible = borrow.n_executed();
        self.candidates.clear();
        self.candidates.extend(borrow.candidate_transactions().cloned());
    }

    // /// Returns when the block content has changed. Returns immediately if the view is
    // /// already outdated. The view is not updated; you need to call [`Self::refresh`] or
    // /// [`Self::refresh_with_candidates`] when this function returns.
    // pub(crate) async fn wait_until_outdated(&mut self) {
    //     self.block_content.changed().await.expect("Channel unexpectedly closed");
    //     self.block_content.mark_changed();
    // }

    /// Wait for the next transaction. When we're up to date with all executed transactions, this
    /// will start returning candidate transactions. When a new executed transaction appears, this
    /// will clear the visible candidate transactions.
    pub async fn wait_next_tx(&mut self) -> PreconfirmedBlockChange {
        loop {
            {
                let borrow = self.block_content.borrow();
                // New preconfirmed transactions
                if borrow.n_executed() > self.n_txs_visible {
                    let transaction_index = self.n_txs_visible as u64;
                    self.n_txs_visible += 1;
                    return PreconfirmedBlockChange::NewPreconfirmed {
                        transaction_index,
                        // Clear all candidates.
                        removed_candidates: mem::take(&mut self.candidates),
                    };
                }
                // New candidate transaction
                if let Some(candidate) = borrow.candidate_transactions().nth(self.candidates.len()) {
                    let transaction_index = (self.n_txs_visible + self.candidates.len()) as u64;
                    self.candidates.push(candidate.clone());
                    return PreconfirmedBlockChange::NewCandidate { transaction_index };
                };
            }
            self.block_content.changed().await.expect("Channel unexpectedly closed");
        }
    }

    pub fn num_executed_transactions(&self) -> usize {
        self.n_txs_visible
    }

    pub fn header(&self) -> &PreconfirmedHeader {
        &self.block.header
    }

    pub fn block_number(&self) -> u64 {
        self.block.header.block_number
    }

    pub fn get_block_info(&self) -> MadaraPreconfirmedBlockInfo {
        MadaraPreconfirmedBlockInfo {
            header: self.block.header.clone(),
            tx_hashes: self
                .borrow_content()
                .executed_transactions()
                .map(|c| *c.transaction.receipt.transaction_hash())
                .collect(),
        }
    }

    pub fn get_transaction(&self, tx_index: u64) -> Option<TransactionWithReceipt> {
        let Some(tx_index) = usize::try_from(tx_index).ok() else { return None };
        self.borrow_content().executed_transactions().nth(tx_index).map(|tx| &tx.transaction).cloned()
    }

    pub fn get_block_transactions(&self, bounds: impl std::ops::RangeBounds<u64>) -> Vec<TransactionWithReceipt> {
        let (from_tx_index, to_take) = super::normalize_transactions_range(bounds);
        self.borrow_content()
            .executed_transactions()
            .skip(from_tx_index)
            .take(to_take)
            .map(|tx| tx.transaction.clone())
            .collect()
    }

    /// Create an aggregated state diff of the preconfirmed block.
    /// The resulting state diff is normalized: when merging state maps together, if for example a transaction changed a
    /// given storage key to new value, and a following transaction change it back to the original value - the normalized state diff
    /// should not preserve the storage key, since it ends up not being changed in the end. This function does this normalization,
    /// by looking up every key in the backend and only keep the changed ones.
    pub fn get_normalized_state_diff(&self) -> Result<StateDiff> {
        fn sorted_by_key<T, K: Ord, F: FnMut(&T) -> K>(mut vec: Vec<T>, f: F) -> Vec<T> {
            vec.sort_by_key(f);
            vec
        }

        let view_on_parent_block = self.state_view_on_parent();
        let borrow = self.borrow_content();

        // Filter and group storage diffs.
        // HashMap<(Addr, K), V> => HashMap<Addr, Vec<(K, V)>>

        let storage_diffs = borrow.executed_transactions().flat_map(|tx| &tx.state_diff.storage).try_fold(
            HashMap::<Felt, Vec<StorageEntry>>::new(),
            |mut acc, ((addr, key), value)| -> anyhow::Result<_> {
                let previous_value = view_on_parent_block.get_contract_storage(addr, key)?.unwrap_or(Felt::ZERO);
                if &previous_value != value {
                    // only keep changed keys.
                    acc.entry(*addr).or_default().push(StorageEntry { key: *key, value: *value });
                }
                Ok(acc)
            },
        )?;
        // Map into a sorted vec.
        let storage_diffs: Vec<ContractStorageDiffItem> = sorted_by_key(
            storage_diffs
                .into_iter()
                .map(|(address, entries)| ContractStorageDiffItem {
                    address,
                    storage_entries: sorted_by_key(entries, |entry| entry.key),
                })
                .collect(),
            |entry| entry.address,
        );

        // We shouldn't need to check the backend for duplicate here: you can't redeclare a class (except on very early mainnet blocks, not handled here).
        let (declared_classes, deprecated_declared_classes): (Vec<DeclaredClassItem>, Vec<Felt>) =
            borrow.executed_transactions().flat_map(|tx| tx.declared_class.as_ref()).partition_map(|class| {
                let class_hash = *class.class_hash();
                if let Some(sierra_class) = class.as_sierra() {
                    // Sierra
                    Either::Left(DeclaredClassItem {
                        class_hash,
                        compiled_class_hash: sierra_class.info.compiled_class_hash,
                    })
                } else {
                    // Legacy
                    Either::Right(class_hash)
                }
            });
        let (declared_classes, deprecated_declared_classes) = (
            sorted_by_key(declared_classes, |entry| entry.class_hash),
            sorted_by_key(deprecated_declared_classes, |class_hash| *class_hash),
        );

        // Same here: duplicate is not possible for nonces.
        let nonces = sorted_by_key(
            borrow
                .executed_transactions()
                .flat_map(|tx| &tx.state_diff.nonces)
                .map(|(contract_address, nonce)| NonceUpdate { contract_address: *contract_address, nonce: *nonce })
                .collect(),
            |entry| entry.contract_address,
        );

        // Differentiate between: newly deployed contract, replaced class entry. Remove entries where no change happened.
        let (deployed_contracts, replaced_classes): (Vec<DeployedContractItem>, Vec<ReplacedClassItem>) =
            borrow.executed_transactions().flat_map(|tx| &tx.state_diff.contract_class_hashes).try_fold(
                (Vec::new(), Vec::new()),
                |(mut deployed, mut replaced), (contract_address, class_hash)| -> anyhow::Result<_> {
                    let previous_class_hash =
                        view_on_parent_block.get_contract_class_hash(contract_address)?.unwrap_or(Felt::ZERO);

                    if previous_class_hash == Felt::ZERO {
                        // Newly deployed contract
                        deployed.push(DeployedContractItem { address: *contract_address, class_hash: *class_hash });
                    } else if &previous_class_hash != class_hash {
                        // Replaced class
                        replaced
                            .push(ReplacedClassItem { contract_address: *contract_address, class_hash: *class_hash });
                    }

                    Ok((deployed, replaced))
                },
            )?;
        let (deployed_contracts, replaced_classes) = (
            sorted_by_key(deployed_contracts, |entry| entry.address),
            sorted_by_key(replaced_classes, |entry| entry.contract_address),
        );

        Ok(StateDiff {
            storage_diffs,
            deprecated_declared_classes,
            declared_classes,
            nonces,
            deployed_contracts,
            replaced_classes,
        })
    }

    /// Get the full block with all classes, and normalize state diffs.
    pub fn get_full_block_with_classes(&self) -> Result<(PreconfirmedFullBlock, Vec<ConvertedClass>)> {
        let header = self.block.header.clone();

        // We don't care about the candidate transactions.
        let mut executed_transactions: Vec<_> = self.block.content.borrow().executed_transactions().cloned().collect();

        let state_diff = self.get_normalized_state_diff().context("Creating normalized state diff")?;
        let classes: Vec<_> = executed_transactions.iter_mut().filter_map(|tx| tx.declared_class.take()).collect();
        let transactions: Vec<_> = executed_transactions.into_iter().map(|tx| tx.transaction.clone()).collect();
        let events = transactions
            .iter()
            .flat_map(|tx| {
                tx.receipt
                    .events()
                    .iter()
                    .cloned()
                    .map(|event| EventWithTransactionHash { transaction_hash: *tx.receipt.transaction_hash(), event })
            })
            .collect();

        Ok((PreconfirmedFullBlock { header, state_diff, transactions, events }, classes))
    }
}

/// A notification of a block change on the preconfirmed block.
#[derive(Debug)]
pub enum PreconfirmedBlockChange {
    NewPreconfirmed { transaction_index: u64, removed_candidates: Vec<TransactionWithHash> },
    NewCandidate { transaction_index: u64 },
}

impl PreconfirmedBlockChange {
    pub fn transaction_index(&self) -> u64 {
        match self {
            Self::NewPreconfirmed { transaction_index, .. } => *transaction_index,
            Self::NewCandidate { transaction_index } => *transaction_index,
        }
    }
    pub fn removed_candidates(&mut self) -> Vec<TransactionWithHash> {
        match self {
            Self::NewPreconfirmed { removed_candidates, .. } => mem::take(removed_candidates),
            _ => vec![],
        }
    }
}
