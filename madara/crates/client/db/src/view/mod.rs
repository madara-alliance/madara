use crate::{
    preconfirmed::{PreconfirmedBlock, PreconfirmedExecutedTransaction, PreconfirmedTransaction},
    prelude::*,
    storage::{EventFilter, MadaraStorageRead, TxIndex},
    MadaraBackend,
};
use mp_block::{EventWithInfo, MadaraMaybePreconfirmedBlockInfo, MadaraPreconfirmedBlockInfo, TransactionWithReceipt};
use mp_class::{ClassInfo, CompiledSierra};
use mp_convert::Felt;
use mp_state_update::StateDiff;
use mp_transactions::TransactionWithHash;
use std::{
    ops::{Bound, RangeBounds},
    sync::Arc,
};

mod anchor;
mod state_diff;

pub use anchor::*;

pub struct MadaraView<D: MadaraStorageRead> {
    pub(crate) backend: Arc<MadaraBackend<D>>,
    pub(crate) anchor: Anchor,
}

impl<D: MadaraStorageRead> Clone for MadaraView<D> {
    fn clone(&self) -> Self {
        Self { backend: self.backend.clone(), anchor: self.anchor.clone() }
    }
}

impl<D: MadaraStorageRead> MadaraView<D> {
    fn new(backend: Arc<MadaraBackend<D>>, anchor: Anchor) -> Self {
        Self { backend, anchor }
    }

    pub fn into_block_view_on(self, block_n: u64) -> Option<MadaraBlockView<D>> {
        self.anchor
            .into_block_anchor()
            .filter(|anchor| block_n <= anchor.block_n())
            .map(|anchor| MadaraBlockView::new(self.backend, anchor))
    }

    pub fn into_block_view(self) -> Option<MadaraBlockView<D>> {
        self.anchor.into_block_anchor().map(|anchor| MadaraBlockView::new(self.backend, anchor))
    }

    fn lookup_preconfirmed_state<V>(
        &self,
        f: impl FnMut((usize, &PreconfirmedExecutedTransaction)) -> Option<V>,
    ) -> Option<V> {
        if let Some(preconfirmed) = self.anchor.preconfirmed() {
            preconfirmed.borrow_content().executed_transactions().enumerate().rev().find_map(f)
        } else {
            None
        }
    }

    pub fn get_contract_storage(&self, contract_address: &Felt, key: &Felt) -> Result<Option<Felt>> {
        let state_diff_key = (*contract_address, *key);
        if let Some(res) = self.lookup_preconfirmed_state(|(_, s)| s.state_diff.storage.get(&state_diff_key).copied()) {
            return Ok(Some(res));
        }
        let Some(block_n) = self.anchor.latest_confirmed_block_n() else { return Ok(None) };
        self.backend.db.get_storage_at(block_n, contract_address, key)
    }

    pub fn get_contract_nonce(&self, contract_address: &Felt) -> Result<Option<Felt>> {
        if let Some(res) = self.lookup_preconfirmed_state(|(_, s)| s.state_diff.nonces.get(contract_address).copied()) {
            return Ok(Some(res));
        }
        let Some(block_n) = self.anchor.latest_confirmed_block_n() else { return Ok(None) };
        self.backend.db.get_contract_nonce_at(block_n, contract_address)
    }

    pub fn get_contract_class_hash(&self, contract_address: &Felt) -> Result<Option<Felt>> {
        if let Some(res) = self.lookup_preconfirmed_state(|(_, s)| s.state_diff.nonces.get(contract_address).copied()) {
            return Ok(Some(res));
        }
        let Some(block_n) = self.anchor.latest_confirmed_block_n() else { return Ok(None) };
        self.backend.db.get_contract_class_hash_at(block_n, contract_address)
    }

    pub fn is_contract_deployed(&self, contract_address: &Felt) -> Result<bool> {
        if self
            .lookup_preconfirmed_state(|(_, s)| {
                if s.state_diff.contract_class_hashes.contains_key(&contract_address) {
                    Some(())
                } else {
                    None
                }
            })
            .is_some()
        {
            return Ok(true);
        }
        let Some(block_n) = self.anchor.latest_confirmed_block_n() else { return Ok(false) };
        self.backend.db.is_contract_deployed_at(block_n, contract_address)
    }

    pub fn get_class(&self, class_hash: &Felt) -> Result<Option<ClassInfo>> {
        if let Some(res) = self.lookup_preconfirmed_state(|(_, s)| {
            s.declared_class.as_ref().filter(|c| c.class_hash() == class_hash).map(|c| c.info())
        }) {
            return Ok(Some(res));
        }
        let Some(block_n) = self.anchor.latest_confirmed_block_n() else { return Ok(None) };
        let Some(class) = self.backend.db.get_class(class_hash)? else { return Ok(None) };
        if class.block_n <= block_n {
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
                .filter(|c| &c.info.compiled_class_hash == compiled_class_hash)
                .map(|c| c.compiled.clone())
        }) {
            return Ok(Some(res));
        }
        let Some(block_n) = self.anchor.latest_confirmed_block_n() else { return Ok(None) };
        let Some(class) = self.backend.db.get_class_compiled(compiled_class_hash)? else { return Ok(None) };
        if class.block_n <= block_n {
            Ok(Some(class.compiled_sierra.into()))
        } else {
            Ok(None)
        }
    }

    pub fn get_transaction_by_hash(&self, tx_hash: &Felt) -> Result<Option<(TxIndex, TransactionWithReceipt)>> {
        if let Some(res) = self.anchor.preconfirmed().and_then(|preconfirmed| {
            preconfirmed.borrow_content().executed_transactions().enumerate().find_map(|(tx_index, tx)| {
                if tx.transaction.receipt.transaction_hash() == tx_hash {
                    Some((
                        TxIndex { block_n: preconfirmed.block.header.block_number, transaction_index: tx_index as _ },
                        tx.transaction.clone(),
                    ))
                } else {
                    None
                }
            })
        }) {
            return Ok(Some(res));
        }

        let Some(on_block_n) = self.anchor.latest_confirmed_block_n() else { return Ok(None) };
        let Some(found) = self.backend.db.find_transaction_hash(tx_hash)? else {
            return Ok(None);
        };

        if found.block_n > on_block_n {
            return Ok(None);
        }

        let tx = self
            .backend
            .db
            .get_transaction(found.block_n, found.transaction_index)?
            .context("Transaction should exist")?;

        Ok(Some((found, tx)))
    }

    pub fn find_transaction_hash(&self, tx_hash: &Felt) -> Result<Option<TxIndex>> {
        if let Some(res) = self.anchor.preconfirmed().and_then(|preconfirmed| {
            preconfirmed.borrow_content().executed_transactions().enumerate().find_map(|(tx_index, tx)| {
                if tx.transaction.receipt.transaction_hash() == tx_hash {
                    Some(TxIndex { block_n: preconfirmed.block.header.block_number, transaction_index: tx_index as _ })
                } else {
                    None
                }
            })
        }) {
            return Ok(Some(res));
        }

        let Some(on_block_n) = self.anchor.latest_confirmed_block_n() else { return Ok(None) };
        let Some(found) = self.backend.db.find_transaction_hash(tx_hash)? else {
            return Ok(None);
        };

        if found.block_n > on_block_n {
            return Ok(None);
        }

        Ok(Some(found))
    }

    pub fn get_transaction(&self, block_n: u64, tx_index: u64) -> Result<Option<TransactionWithReceipt>> {
        let Ok(tx_index_i) = usize::try_from(tx_index) else { return Ok(None) };
        if let Some(res) = self.anchor.preconfirmed().filter(|p| p.block.header.block_number == block_n) {
            return Ok(res.borrow_content().executed_transactions().nth(tx_index_i).map(|tx| tx.transaction.clone()));
        }
        if !self.anchor.latest_confirmed_block_n().is_some_and(|on_block_n| on_block_n >= block_n) {
            return Ok(None);
        }

        self.backend.db.get_transaction(block_n, tx_index)
    }

    pub fn get_events(&self, _filter: EventFilter) -> Result<Vec<EventWithInfo>> {
        // let mut events = if let Some(closed_block_n) = self.anchor.on_block_n() {
        //     let mut filter = filter.clone();
        //     filter.end_block = cmp::min(closed_block_n, filter.end_block);
        //     if filter.end_block >= filter.start_block {
        //         self.backend.db.get_events(filter)?
        //     } else {
        //         vec![]
        //     }
        // } else {
        //     vec![]
        // };

        // if events.len() < filter.max_events && filter.end_block >= filter.start_block {
        //     if let Some(preconfirmed) = self.anchor.preconfirmed().filter(|p| p.block.block_n >= filter.end_block) {
        //         let skip = if preconfirmed.block.block_n == filter.start_block {
        //             filter.start_event_index
        //         } else {
        //             0
        //         };
        //         events.extend(preconfirmed.content().events_with_info().skip(skip).filter(|ev| filter.matches(ev)).take(events.len() - filter.max_events));
        //     }
        // }
        // events
        Ok(vec![])
    }
}

pub struct MadaraBlockView<D: MadaraStorageRead> {
    pub(crate) backend: Arc<MadaraBackend<D>>,
    pub(crate) anchor: BlockAnchor,
}

impl<D: MadaraStorageRead> MadaraBlockView<D> {
    pub(crate) fn new(backend: Arc<MadaraBackend<D>>, anchor: BlockAnchor) -> Self {
        Self { backend, anchor }
    }
}

// derive(Clone) will put a D: Clone bounds which we don't want, so we have to implement clone by hand :(
impl<D: MadaraStorageRead> Clone for MadaraBlockView<D> {
    fn clone(&self) -> Self {
        Self { backend: self.backend.clone(), anchor: self.anchor.clone() }
    }
}

impl<D: MadaraStorageRead> MadaraBlockView<D> {
    pub fn is_preconfirmed(&self) -> bool {
        self.anchor.as_preconfirmed().is_some()
    }

    pub fn block_n(&self) -> u64 {
        self.anchor.block_n()
    }

    pub fn is_on_l1(&self) -> bool {
        !self.anchor.is_preconfirmed()
            && self.backend.latest_l1_confirmed_block_n().is_some_and(|last_on_l1| self.block_n() <= last_on_l1)
    }

    pub fn get_block_info(&self) -> Result<MadaraMaybePreconfirmedBlockInfo> {
        match &self.anchor {
            BlockAnchor::Preconfirmed(preconfirmed) => {
                Ok(MadaraMaybePreconfirmedBlockInfo::Preconfirmed(MadaraPreconfirmedBlockInfo {
                    header: preconfirmed.block.header.clone(),
                    tx_hashes: preconfirmed
                        .borrow_content()
                        .executed_transactions()
                        .map(|c| *c.transaction.receipt.transaction_hash())
                        .collect(),
                }))
            }
            BlockAnchor::Confirmed(block_n) => Ok(self
                .backend
                .db
                .get_block_info(*block_n)?
                .map(MadaraMaybePreconfirmedBlockInfo::Closed)
                .context("Block info should be found")?),
        }
    }

    pub fn get_state_diff(&self) -> Result<StateDiff> {
        if let Some(preconfirmed) = self.to_preconfirmed() {
            preconfirmed.get_normalized_state_diff()
        } else {
            self.backend.db.get_block_state_diff(self.block_n())?.context("Block state diff should be found")
        }
    }

    pub fn get_transaction(&self, tx_index: u64) -> Result<Option<TransactionWithReceipt>> {
        let Some(tx_index) = usize::try_from(tx_index).ok() else { return Ok(None) };
        match &self.anchor {
            BlockAnchor::Preconfirmed(preconfirmed) => Ok(preconfirmed
                .borrow_content()
                .executed_transactions()
                .nth(tx_index)
                .map(|tx| &tx.transaction)
                .cloned()),
            BlockAnchor::Confirmed(block_n) => Ok(self.backend.db.get_transaction(*block_n, tx_index as u64)?),
        }
    }

    pub fn get_block_transactions(&self, bounds: impl RangeBounds<u64>) -> Result<Vec<TransactionWithReceipt>> {
        let (start, end) = (bounds.start_bound().cloned(), bounds.end_bound().cloned());

        let start_tx_index = match start {
            Bound::Excluded(start) => start.saturating_add(1),
            Bound::Included(start) => start,
            Bound::Unbounded => 0,
        };
        let Some(start_tx_index) = usize::try_from(start_tx_index).ok() else { return Ok(vec![]) };
        let end_tx_index = match end {
            Bound::Excluded(end) => end,
            Bound::Included(end) => end.saturating_add(1),
            Bound::Unbounded => u64::MAX.into(),
        };
        let end_tx_index = usize::try_from(end_tx_index).unwrap_or(usize::MAX);

        let to_take = end_tx_index.saturating_sub(start_tx_index);
        if to_take == 0 {
            return Ok(vec![]);
        }

        match &self.anchor {
            BlockAnchor::Preconfirmed(preconfirmed) => Ok(preconfirmed
                .borrow_content()
                .executed_transactions()
                .skip(start_tx_index)
                .take(end_tx_index)
                .map(|tx| tx.transaction.clone())
                .collect()),
            BlockAnchor::Confirmed(block_n) => Ok(self
                .backend
                .db
                .get_block_transactions(*block_n, start_tx_index as u64)
                .take(to_take)
                .collect::<Result<_, _>>()?),
        }
    }

    /// Make a preconfirmed block view. Returns [`None`] if the view is not on a preconfirmed block.
    pub fn into_preconfirmed(self) -> Option<MadaraPreconfirmedBlockView<D>> {
        self.anchor.into_preconfirmed().map(|anchor| MadaraPreconfirmedBlockView::new(self.backend, anchor))
    }

    /// Make a preconfirmed block view. Returns [`None`] if the view is not on a preconfirmed block.
    pub fn to_preconfirmed(&self) -> Option<MadaraPreconfirmedBlockView<D>> {
        self.anchor
            .as_preconfirmed()
            .cloned()
            .map(|anchor| MadaraPreconfirmedBlockView::new(self.backend.clone(), anchor))
    }
}

pub struct MadaraPreconfirmedBlockView<D: MadaraStorageRead> {
    pub(crate) backend: Arc<MadaraBackend<D>>,
    pub(crate) anchor: PreconfirmedBlockAnchor,
}

impl<D: MadaraStorageRead> Clone for MadaraPreconfirmedBlockView<D> {
    fn clone(&self) -> Self {
        Self { backend: self.backend.clone(), anchor: self.anchor.clone() }
    }
}

impl<D: MadaraStorageRead> MadaraPreconfirmedBlockView<D> {
    pub(crate) fn new(backend: Arc<MadaraBackend<D>>, anchor: PreconfirmedBlockAnchor) -> Self {
        Self { backend, anchor }
    }
}

impl<D: MadaraStorageRead> MadaraPreconfirmedBlockView<D> {
    pub fn into_block_view(self) -> MadaraBlockView<D> {
        MadaraBlockView::new(self.backend, BlockAnchor::Preconfirmed(self.anchor))
    }

    /// Make none of the transactions visible.
    pub fn reset_to_start(&mut self) {
        self.anchor.refresh();
    }

    /// Refresh the view. Added transactions will be visible.
    pub fn refresh(&mut self) {
        self.anchor.refresh();
    }

    /// Make the new transactions visible. Candidate transactions will also be visible.
    pub fn refresh_with_candidates(&mut self) {
        self.anchor.refresh_with_candidates();
    }

    /// Returns when the block content has changed. Returns immediately if the view is
    /// already outdated. The view is not updated; you need to call [`Self::refresh`] or
    /// [`Self::refresh_with_candidates`] when this function returns.
    pub async fn wait_until_outdated(&mut self) {
        self.anchor.wait_until_outdated().await
    }
    pub fn get_transaction_with_candidate(&self, tx_index: u64) -> Option<PreconfirmedTransaction> {
        self.anchor.block_content.borrow().all_transactions().nth(tx_index.try_into().ok()?).cloned()
    }
    pub fn get_block_transactions_with_candidates(
        &self,
        bounds: impl RangeBounds<u64>,
    ) -> Vec<PreconfirmedTransaction> {
        let (start, end) = (bounds.start_bound().cloned(), bounds.end_bound().cloned());

        let start_tx_index = match start {
            Bound::Excluded(start) => start.saturating_add(1),
            Bound::Included(start) => start,
            Bound::Unbounded => 0,
        };
        let Some(start_tx_index) = usize::try_from(start_tx_index).ok() else { return vec![] };
        let end_tx_index = match end {
            Bound::Excluded(end) => end,
            Bound::Included(end) => end.saturating_add(1),
            Bound::Unbounded => u64::MAX.into(),
        };
        let end_tx_index = usize::try_from(end_tx_index).unwrap_or(usize::MAX);

        let to_take = end_tx_index.saturating_sub(start_tx_index);
        if to_take == 0 {
            return vec![];
        }
        self.anchor
            .borrow_content()
            .executed_transactions()
            .cloned()
            .map(PreconfirmedTransaction::Executed)
            .chain(self.anchor.candidates.iter().cloned().map(PreconfirmedTransaction::Candidate))
            .skip(start_tx_index)
            .take(end_tx_index)
            .collect()
    }

    pub fn get_candidates(&self) -> impl Iterator<Item = &TransactionWithHash> {
        self.anchor.candidates.iter()
    }

    pub fn n_executed(&self) -> usize {
        self.anchor.n_executed()
    }

    pub async fn wait_for_new_tx(&mut self) -> PreconfirmedBlockChange {
        self.anchor.wait_next_tx().await
    }

    /// Anchor on the previous block.
    pub fn view_on_parent(&self) -> MadaraView<D> {
        MadaraView::new(
            self.backend.clone(),
            Anchor::new_on_confirmed(self.anchor.block.header.block_number.checked_sub(1)),
        )
    }
}

impl<D: MadaraStorageRead> MadaraBackend<D> {
    pub(crate) fn view_on_anchor(self: &Arc<Self>, anchor: Anchor) -> MadaraView<D> {
        MadaraView::new(self.clone(), anchor)
    }
    pub(crate) fn block_view_on_anchor(self: &Arc<Self>, anchor: BlockAnchor) -> MadaraBlockView<D> {
        MadaraBlockView::new(self.clone(), anchor)
    }
    pub(crate) fn preconfirmed_view_on_anchor(
        self: &Arc<Self>,
        anchor: PreconfirmedBlockAnchor,
    ) -> MadaraPreconfirmedBlockView<D> {
        MadaraPreconfirmedBlockView::new(self.clone(), anchor)
    }

    pub fn view_on(self: &Arc<Self>, anchor: impl IntoAnchor) -> Result<Option<MadaraView<D>>> {
        Ok(anchor.into_anchor(self.as_ref())?.map(|anchor| self.view_on_anchor(anchor)))
    }
    pub fn view_on_latest_confirmed(self: &Arc<Self>) -> MadaraView<D> {
        self.view_on_anchor(Anchor::new_on_confirmed(self.latest_confirmed_block_n()))
    }
    /// May return a view on a fake preconfirmed block if none was found.
    pub fn view_on_preconfirmed(self: &Arc<Self>) -> MadaraView<D> {
        self.view_on_anchor(Anchor::new_on_preconfirmed(self.get_preconfirmed_or_fake().clone()))
    }

    pub fn block_view_on_confirmed(self: &Arc<Self>, block_n: u64) -> MadaraBlockView<D> {
        self.block_view_on_anchor(BlockAnchor::new_on_confirmed(block_n))
    }
    pub fn block_view_on(self: &Arc<Self>, anchor: impl IntoAnchor) -> Result<Option<MadaraBlockView<D>>> {
        Ok(anchor.into_block_anchor(self.as_ref())?.map(|anchor| self.block_view_on_anchor(anchor)))
    }
    pub fn block_view_on_latest_confirmed(self: &Arc<Self>) -> Option<MadaraBlockView<D>> {
        Some(self.block_view_on_anchor(BlockAnchor::new_on_confirmed(self.latest_confirmed_block_n()?)))
    }
    /// May return a view on a fake preconfirmed block if none was found.
    pub fn block_view_on_preconfirmed(self: &Arc<Self>) -> MadaraBlockView<D> {
        self.block_view_on_anchor(BlockAnchor::new_on_preconfirmed(self.get_preconfirmed_or_fake().clone()))
    }

    /// Returns a fake preconfirmed block if none was found.
    pub fn get_preconfirmed_or_fake(&self) -> Arc<PreconfirmedBlock> {
        let anchor = self.chain_tip.borrow();
        if let Some(preconfirmed) = anchor.preconfirmed() {
            preconfirmed.block.clone()
        } else {
            // Fake preconfirmed
            todo!()
        }
    }
}
