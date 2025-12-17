use crate::{
    preconfirmed::PreconfirmedExecutedTransaction, prelude::*, rocksdb::RocksDBStorage,
    view::ExecutedTransactionWithBlockView, EventFilter, StorageTxIndex,
};
use mp_block::{EventWithInfo, TransactionWithReceipt};
use mp_class::{ClassInfo, CompiledSierra, ConvertedClass, LegacyConvertedClass, SierraConvertedClass};
use std::cmp;

#[derive(Debug)]
pub enum MadaraStateView<D: MadaraStorageRead = RocksDBStorage> {
    /// Pre-genesis empty state. No blocks will be visible, and every query will resolve as not found.
    Empty(Arc<MadaraBackend<D>>),
    /// Queries will be resolfed on top of the given block: its state will be visible, but no later state will.
    OnBlock(MadaraBlockView<D>),
}

// derive(Clone) will put a D: Clone bounds which we don't want, so we have to implement clone by hand :(
impl<D: MadaraStorageRead> Clone for MadaraStateView<D> {
    fn clone(&self) -> Self {
        match self {
            Self::Empty(backend) => Self::Empty(backend.clone()),
            Self::OnBlock(view) => Self::OnBlock(view.clone()),
        }
    }
}

impl<D: MadaraStorageRead> fmt::Display for MadaraStateView<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty(_) => write!(f, "[Empty pre-genesis state]"),
            Self::OnBlock(b) => write!(f, "{b}"),
        }
    }
}

impl<D: MadaraStorageRead, T: Into<MadaraBlockView<D>>> From<T> for MadaraStateView<D> {
    fn from(value: T) -> Self {
        Self::OnBlock(value.into())
    }
}

impl<D: MadaraStorageRead> MadaraStateView<D> {
    pub(super) fn on_confirmed_or_empty(backend: Arc<MadaraBackend<D>>, block_number: Option<u64>) -> Self {
        match block_number {
            Some(block_number) => MadaraConfirmedBlockView::new(backend, block_number).into(),
            None => Self::Empty(backend),
        }
    }

    pub fn backend(&self) -> &Arc<MadaraBackend<D>> {
        match self {
            Self::Empty(backend) => backend,
            Self::OnBlock(view) => view.backend(),
        }
    }

    /// Returns a view on a confirmed block. This view is used to query content from that block.
    /// Returns [`None`] if the block number is not yet confirmed.
    pub fn block_view_on_latest(&self) -> Option<&MadaraBlockView<D>> {
        match self {
            Self::OnBlock(view) => Some(view),
            _ => None,
        }
    }

    /// Returns a view on the latest confirmed block. This view is used to query content from that block.
    pub fn view_on_latest_confirmed(&self) -> MadaraStateView<D> {
        match self {
            Self::OnBlock(_) => Self::on_confirmed_or_empty(self.backend().clone(), self.latest_confirmed_block_n()),
            _ => Self::Empty(self.backend().clone()),
        }
    }
    /// Returns a view on the latest confirmed block. This view is used to query content from that block.
    /// Returns [`None`] if the block number is not yet confirmed or does not exist.
    pub fn view_on_confirmed(&self, block_number: u64) -> Option<MadaraStateView<D>> {
        self.block_view_on_confirmed(block_number).map(Into::into)
    }

    /// Returns a view on the latest confirmed block. This view is used to query content from that block.
    /// Returns [`None`] if the block number is not yet confirmed or does not exist.
    pub fn block_view_on_latest_confirmed(&self) -> Option<MadaraConfirmedBlockView<D>> {
        self.latest_confirmed_block_n()
            .map(|block_number| MadaraConfirmedBlockView::new(self.backend().clone(), block_number))
    }

    /// Returns a view on a confirmed block. This view is used to query content from that block.
    /// Returns [`None`] if the block number is not yet confirmed.
    pub fn block_view_on_confirmed(&self, block_number: u64) -> Option<MadaraConfirmedBlockView<D>> {
        self.latest_confirmed_block_n()
            .filter(|n| n >= &block_number)
            .map(|_| MadaraConfirmedBlockView::new(self.backend().clone(), block_number))
    }

    /// Latest confirmed block_n visible from this view.
    /// Returns [`None`] if no confirmed blocks are visible from this view.
    pub fn latest_confirmed_block_n(&self) -> Option<u64> {
        match self {
            Self::Empty(_) => None,
            Self::OnBlock(MadaraBlockView::Preconfirmed(view)) => view.block_number().checked_sub(1),
            Self::OnBlock(MadaraBlockView::Confirmed(view)) => Some(view.block_number()),
        }
    }

    /// Latest block_n visible from this view, which may be a pre-confirmed block.
    /// Returns [`None`] if no blocks are visible from this view.
    pub fn latest_block_n(&self) -> Option<u64> {
        match self {
            Self::Empty(_) => None,
            Self::OnBlock(view) => Some(view.block_number()),
        }
    }

    /// Returns `true` if a preconfirmed block is visible from this view.
    pub fn has_preconfirmed_block(&self) -> bool {
        match self {
            Self::Empty(_) => false,
            Self::OnBlock(view) => view.is_preconfirmed(),
        }
    }

    /// Latest confirmed block_n on l1 visible from this view.
    pub fn latest_l1_confirmed_block_n(&self) -> Option<u64> {
        match self {
            Self::Empty(_) => None,
            Self::OnBlock(view) => view
                .backend()
                .latest_l1_confirmed_block_n()
                .map(|block_number| std::cmp::min(view.block_number(), block_number)),
        }
    }

    // STATE QUERIES

    /// Get a confirmed block by hash.
    pub fn find_block_by_hash(&self, block_hash: &Felt) -> Result<Option<u64>> {
        let Some(latest_confirmed) = self.latest_confirmed_block_n() else { return Ok(None) };
        Ok(self.backend().db.find_block_hash(block_hash)?.filter(|found_block_n| found_block_n >= &latest_confirmed))
    }

    fn lookup_preconfirmed_state<V>(
        &self,
        f: impl FnMut((usize, &PreconfirmedExecutedTransaction)) -> Option<V>,
    ) -> Option<V> {
        if let Some(preconfirmed) = self.block_view_on_latest().and_then(|v| v.as_preconfirmed()) {
            preconfirmed.borrow_content().executed_transactions().enumerate().rev().find_map(f)
        } else {
            None
        }
    }

    pub fn get_contract_storage(&self, contract_address: &Felt, key: &Felt) -> Result<Option<Felt>> {
        let state_diff_key = (*contract_address, *key);
        if let Some(res) =
            self.lookup_preconfirmed_state(|(_, s)| s.state_diff.storage_diffs.get(&state_diff_key).copied())
        {
            return Ok(Some(res));
        }
        let Some(block_n) = self.latest_confirmed_block_n() else { return Ok(None) };
        self.backend().db.get_storage_at(block_n, contract_address, key)
    }

    pub fn get_contract_nonce(&self, contract_address: &Felt) -> Result<Option<Felt>> {
        if let Some(res) = self.lookup_preconfirmed_state(|(_, s)| s.state_diff.nonces.get(contract_address).copied()) {
            return Ok(Some(res));
        }
        let Some(block_n) = self.latest_confirmed_block_n() else { return Ok(None) };
        self.backend().db.get_contract_nonce_at(block_n, contract_address)
    }

    pub fn get_contract_class_hash(&self, contract_address: &Felt) -> Result<Option<Felt>> {
        if let Some(res) =
            self.lookup_preconfirmed_state(|(_, s)| s.state_diff.contract_class_hashes.get(contract_address).copied())
        {
            return Ok(Some(*res.class_hash()));
        }
        let Some(block_n) = self.latest_confirmed_block_n() else { return Ok(None) };
        self.backend().db.get_contract_class_hash_at(block_n, contract_address)
    }

    pub fn is_contract_deployed(&self, contract_address: &Felt) -> Result<bool> {
        if self
            .lookup_preconfirmed_state(|(_, s)| {
                if s.state_diff.contract_class_hashes.contains_key(contract_address) {
                    Some(())
                } else {
                    None
                }
            })
            .is_some()
        {
            return Ok(true);
        }
        let Some(block_n) = self.latest_confirmed_block_n() else { return Ok(false) };
        self.backend().db.is_contract_deployed_at(block_n, contract_address)
    }

    pub fn get_class_info(&self, class_hash: &Felt) -> Result<Option<ClassInfo>> {
        if let Some(res) = self.lookup_preconfirmed_state(|(_, s)| {
            s.declared_class.as_ref().filter(|c| c.class_hash() == class_hash).map(|c| c.info())
        }) {
            return Ok(Some(res));
        }
        let Some(block_n) = self.latest_confirmed_block_n() else { return Ok(None) };
        let Some(class) = self.backend().db.get_class(class_hash)? else { return Ok(None) };
        if class.block_number <= block_n {
            Ok(Some(class.class_info))
        } else {
            Ok(None)
        }
    }

    pub fn get_class_compiled(&self, compiled_class_hash: &Felt) -> Result<Option<Arc<CompiledSierra>>> {
        if let Some(res) = self.lookup_preconfirmed_state(|(_, s)| {
            s.declared_class
                .as_ref()
                .and_then(|c| c.as_sierra())
                .filter(|c| {
                    // Check canonical hash (v2 if present, else v1)
                    let canonical = c.info.compiled_class_hash_v2.or(c.info.compiled_class_hash);
                    canonical.as_ref() == Some(compiled_class_hash)
                })
                .map(|c| c.compiled.clone())
        }) {
            return Ok(Some(res));
        }
        let Some(block_n) = self.latest_confirmed_block_n() else { return Ok(None) };
        let Some(class) = self.backend().db.get_class_compiled(compiled_class_hash)? else { return Ok(None) };
        if class.block_number <= block_n {
            Ok(Some(class.compiled_sierra))
        } else {
            Ok(None)
        }
    }

    pub fn get_class_info_and_compiled(&self, class_hash: &Felt) -> Result<Option<ConvertedClass>> {
        let Some(class_info) = self.get_class_info(class_hash).context("Getting class info from class_hash")? else {
            return Ok(None);
        };
        let compiled = match class_info {
            ClassInfo::Sierra(sierra_class_info) => {
                // Try v2 hash first, then fall back to v1 (for migrated classes where
                // the compiled Sierra may still be stored under the v1 hash).
                let compiled = if let Some(hash) = sierra_class_info.compiled_class_hash_v2 {
                    self.get_class_compiled(&hash).context("Getting class compiled from v2 hash")?
                } else if let Some(hash) = sierra_class_info.compiled_class_hash {
                    self.get_class_compiled(&hash).context("Getting class compiled from v1 hash")?
                } else {
                    None
                };

                ConvertedClass::Sierra(SierraConvertedClass {
                    class_hash: *class_hash,
                    compiled: compiled.context("Class info found, compiled class should be found")?,
                    info: sierra_class_info,
                })
            }
            ClassInfo::Legacy(legacy_class_info) => {
                ConvertedClass::Legacy(LegacyConvertedClass { class_hash: *class_hash, info: legacy_class_info })
            }
        };
        Ok(Some(compiled))
    }

    /// This will not return candidate transactions.
    pub fn find_transaction_by_hash(&self, tx_hash: &Felt) -> Result<Option<ExecutedTransactionWithBlockView<D>>> {
        if let Some(res) = self.block_view_on_latest().and_then(|v| v.as_preconfirmed()).and_then(|preconfirmed| {
            preconfirmed.borrow_content().executed_transactions().enumerate().find_map(|(tx_index, tx)| {
                if tx.transaction.receipt.transaction_hash() == tx_hash {
                    Some(ExecutedTransactionWithBlockView {
                        transaction_index: tx_index as _,
                        block: preconfirmed.clone().into(),
                    })
                } else {
                    None
                }
            })
        }) {
            return Ok(Some(res));
        }

        let Some(on_block_n) = self.latest_confirmed_block_n() else { return Ok(None) };
        let Some(StorageTxIndex { block_number, transaction_index }) =
            self.backend().db.find_transaction_hash(tx_hash)?
        else {
            return Ok(None);
        };

        if block_number > on_block_n {
            return Ok(None);
        }

        Ok(Some(ExecutedTransactionWithBlockView {
            transaction_index,
            block: MadaraConfirmedBlockView::new(self.backend().clone(), block_number).into(),
        }))
    }

    pub fn get_executed_transaction(&self, block_number: u64, tx_index: u64) -> Result<Option<TransactionWithReceipt>> {
        let Ok(tx_index_i) = usize::try_from(tx_index) else { return Ok(None) };
        if let Some(preconfirmed) =
            self.block_view_on_latest().and_then(|v| v.as_preconfirmed()).filter(|p| p.block_number() == block_number)
        {
            return Ok(preconfirmed
                .borrow_content()
                .executed_transactions()
                .nth(tx_index_i)
                .map(|tx| tx.transaction.clone()));
        }
        if self.latest_confirmed_block_n().is_none_or(|on_block_n| on_block_n < block_number) {
            return Ok(None);
        }

        self.backend().db.get_transaction(block_number, tx_index)
    }

    pub fn get_events(&self, filter: EventFilter) -> Result<Vec<EventWithInfo>> {
        // First, get the events from the confirmed blocks in DB.
        let mut events = if let Some(latest_confirmed) =
            self.latest_confirmed_block_n().filter(|c| filter.start_block <= *c)
        {
            // only return up to latest visible confirmed block
            self.backend()
                .db
                .get_events(EventFilter { end_block: cmp::min(latest_confirmed, filter.end_block), ..filter.clone() })?
        } else {
            vec![]
        };

        // Then, append events from the preconfirmed block when requested.
        if let Some(preconfirmed) = self
            .block_view_on_latest()
            .and_then(|v| v.as_preconfirmed())
            .filter(|p| p.block_number() >= filter.start_block && p.block_number() <= filter.end_block)
        {
            let skip_events =
                if preconfirmed.block_number() == filter.start_block { filter.start_event_index } else { 0 };

            events.extend(
                preconfirmed
                    .borrow_content()
                    .executed_transactions()
                    .enumerate()
                    .flat_map(|(tx_index, tx)| {
                        tx.transaction.receipt.events().iter().enumerate().map(move |(event_index, event)| {
                            EventWithInfo {
                                event: event.clone(),
                                block_number: preconfirmed.block_number(),
                                block_hash: None,
                                transaction_hash: *tx.transaction.receipt.transaction_hash(),
                                transaction_index: tx_index as u64,
                                event_index_in_block: event_index as u64,
                                in_preconfirmed: true,
                            }
                        })
                    })
                    .filter(|event| filter.matches(&event.event))
                    .skip(skip_events)
                    .take(filter.max_events - events.len()),
            )
        }
        Ok(events)
    }
}
