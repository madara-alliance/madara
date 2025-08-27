use crate::{
    preconfirmed::{PreconfirmedBlock, PreconfirmedBlockInner, PreconfirmedExecutedTransaction},
    prelude::*,
    rocksdb::RocksDBStorage,
};
use mp_block::{
    header::PreconfirmedHeader, MadaraPreconfirmedBlockInfo, FullBlockWithoutCommitments, TransactionWithReceipt,
};
use mp_class::ConvertedClass;
use mp_receipt::EventWithTransactionHash;
use mp_state_update::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateDiff,
    StorageEntry,
};
use mp_transactions::validated::ValidatedTransaction;
use std::fmt;

/// Lock guard on the content of a preconfirmed block. Only the first `n_txs_visible` executed transactions
/// are visible.
pub struct PreconfirmedContentRef<'a, D: MadaraStorageRead> {
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
    /// This vec is only filled when using `refresh_with_candidates`.
    candidates: Vec<Arc<ValidatedTransaction>>,
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
    pub fn block(&self) -> &Arc<PreconfirmedBlock> {
        &self.block
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
    pub fn borrow_content(&self) -> PreconfirmedContentRef<'_, D> {
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

    /// Returns when the block content has changed. Returns immediately if the view is
    /// already outdated. The view is not updated; you need to call [`Self::refresh`] or
    /// [`Self::refresh_with_candidates`] when this function returns.
    pub async fn wait_until_outdated(&mut self) {
        self.block_content.changed().await.expect("Channel unexpectedly closed");
        self.block_content.mark_changed();
    }

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

    pub fn get_executed_transaction(&self, transaction_index: u64) -> Option<TransactionWithReceipt> {
        let Some(transaction_index) = usize::try_from(transaction_index).ok() else { return None };
        self.borrow_content().executed_transactions().nth(transaction_index).map(|tx| &tx.transaction).cloned()
    }

    pub fn get_executed_transactions(&self, bounds: impl std::ops::RangeBounds<u64>) -> Vec<TransactionWithReceipt> {
        let (from_tx_index, to_take) = super::normalize_transactions_range(bounds);
        self.borrow_content()
            .executed_transactions()
            .skip(from_tx_index)
            .take(to_take)
            .map(|tx| tx.transaction.clone())
            .collect()
    }

    /// Candidate transactions. Most of the time, we don't care about those, so this vec is empty.
    /// This will be empty unless filled by using [`Self::refresh_with_candidates`].
    pub fn candidate_transactions(&self) -> &[Arc<ValidatedTransaction>] {
        &self.candidates
    }

    /// Create an aggregated state diff of the preconfirmed block.
    /// The resulting state diff is normalized: when merging state maps together, if for example a transaction changed a
    /// given storage key to new value, and a following transaction change it back to the original value - the normalized state diff
    /// should not preserve the storage key, since it ends up not being changed in the end. This function does this normalization,
    /// by looking up every key in the backend and only keep the changed ones.
    ///
    /// This function will also add the block hash entry for `block_n-10` on the 0x1 contract address.
    pub fn get_normalized_state_diff(&self) -> Result<StateDiff> {
        fn sorted_by_key<T, K: Ord, F: FnMut(&T) -> K>(mut vec: Vec<T>, f: F) -> Vec<T> {
            vec.sort_by_key(f);
            vec
        }

        let mut storage_diffs: HashMap<Felt, HashMap<Felt, Felt>> = Default::default();

        // Add the block hash entry for `block_n-10` on the 0x1 contract address.
        if let Some(block_number) = self.block_number().checked_sub(10) {
            let block_hash = self
                .backend
                .block_view_on_confirmed(block_number)
                .with_context(|| format!("Block at height {block_number} should be in the backend"))?
                .get_block_info()?
                .block_hash;

            storage_diffs.entry(Felt::ONE).or_default().insert(Felt::from(block_number), block_hash);
        }

        // Aggregate all transaction state diffs.

        let mut contract_class_hashes: HashMap<Felt, Felt> = Default::default();
        let mut nonces: Vec<NonceUpdate> = Default::default();
        let mut declared_classes: Vec<DeclaredClassItem> = Default::default();
        let mut old_declared_contracts: Vec<Felt> = Default::default();
        {
            // Get the lock guard on the block content.
            let borrow = self.borrow_content();

            for tx in borrow.executed_transactions() {
                // Storage diffs.
                for ((contract, key), value) in &tx.state_diff.storage {
                    storage_diffs.entry(*contract).or_default().insert(*key, *value);
                }

                // Changed contract class hashes.
                contract_class_hashes.extend(tx.state_diff.contract_class_hashes.iter().map(|(k, v)| (*k, *v)));

                // Nonces.
                nonces.extend(tx.state_diff.nonces.iter().map(|(contract_address, nonce)| NonceUpdate {
                    contract_address: *contract_address,
                    nonce: *nonce,
                }));

                // Classes.
                if let Some(declared_class) = &tx.declared_class {
                    match declared_class {
                        ConvertedClass::Legacy(class) => old_declared_contracts.push(class.class_hash),
                        ConvertedClass::Sierra(class) => declared_classes.push(DeclaredClassItem {
                            class_hash: class.class_hash,
                            compiled_class_hash: class.info.compiled_class_hash,
                        }),
                    }
                }
            }
        } // Lock on the block content is dropped here.

        // Normalize, remove redundant entries & sort.

        let view_on_parent_block = self.state_view_on_parent();

        // Map storage diffs, keeping only changed entries, and sort the result.
        let storage_diffs: Vec<ContractStorageDiffItem> = sorted_by_key(
            storage_diffs
                .into_iter()
                .map(|(address, storage_entries)| {
                    let storage_entries: Vec<StorageEntry> = sorted_by_key(
                        storage_entries
                            .into_iter()
                            .map(|(key, value)| {
                                let previous_value =
                                    view_on_parent_block.get_contract_storage(&address, &key)?.unwrap_or(Felt::ZERO);
                                // Only keep changed keys.
                                Ok((previous_value != value).then_some(StorageEntry { key, value }))
                            })
                            .filter_map_ok(|v| v)
                            .collect::<Result<_>>()?,
                        |entry| entry.key,
                    );

                    // Remove empty entries.
                    if storage_entries.is_empty() {
                        Ok(None)
                    } else {
                        Ok(Some(ContractStorageDiffItem {
                            address,
                            storage_entries: sorted_by_key(storage_entries, |entry| entry.key),
                        }))
                    }
                })
                .filter_map_ok(|v| v)
                .collect::<Result<_>>()?,
            |entry| entry.address,
        );
        // Differentiate between: newly deployed contract, replaced contract class. Remove entries where no change happened.
        let (mut deployed_contracts, mut replaced_classes): (Vec<DeployedContractItem>, Vec<ReplacedClassItem>) =
            Default::default();
        for (address, class_hash) in contract_class_hashes {
            let previous_class_hash = view_on_parent_block.get_contract_class_hash(&address)?.unwrap_or(Felt::ZERO);

            if previous_class_hash == Felt::ZERO {
                // Newly deployed contract
                deployed_contracts.push(DeployedContractItem { address, class_hash });
            } else if previous_class_hash != class_hash {
                // Replaced class
                replaced_classes.push(ReplacedClassItem { contract_address: address, class_hash });
            }
            // Do not include the entry if the class hash has not changed.
        }
        let (deployed_contracts, replaced_classes) = (
            sorted_by_key(deployed_contracts, |entry| entry.address),
            sorted_by_key(replaced_classes, |entry| entry.contract_address),
        );

        // Nonce entries do not need to be checked against the database, since they can never take a previous value.
        // Same with the classes: they can only be declared once.
        let nonces = sorted_by_key(nonces, |entry| entry.contract_address);
        let declared_classes = sorted_by_key(declared_classes, |entry| entry.class_hash);
        let old_declared_contracts = sorted_by_key(old_declared_contracts, |class_hash| *class_hash);

        Ok(StateDiff {
            storage_diffs,
            old_declared_contracts,
            declared_classes,
            nonces,
            deployed_contracts,
            replaced_classes,
        })
    }

    /// Get the full block with all classes, and normalize state diffs.
    pub fn get_full_block_with_classes(&self) -> Result<(FullBlockWithoutCommitments, Vec<ConvertedClass>)> {
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

        Ok((FullBlockWithoutCommitments { header, state_diff, transactions, events }, classes))
    }
}

/// A notification of a block change on the preconfirmed block.
#[derive(Debug)]
pub enum PreconfirmedBlockChange {
    NewPreconfirmed { transaction_index: u64, removed_candidates: Vec<Arc<ValidatedTransaction>> },
    NewCandidate { transaction_index: u64 },
}

impl PreconfirmedBlockChange {
    pub fn transaction_index(&self) -> u64 {
        match self {
            Self::NewPreconfirmed { transaction_index, .. } => *transaction_index,
            Self::NewCandidate { transaction_index } => *transaction_index,
        }
    }
    pub fn removed_candidates(&mut self) -> Vec<Arc<ValidatedTransaction>> {
        match self {
            Self::NewPreconfirmed { removed_candidates, .. } => mem::take(removed_candidates),
            _ => vec![],
        }
    }
}
