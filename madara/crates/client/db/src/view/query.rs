use crate::{
    storage::{EventFilter, MadaraStorageRead, TxIndex},
    prelude::*,
    view::{BlockAnchor, MadaraBackendBlockView, MadaraBackendView, PreconfirmedBlockTransaction},
};
use mp_block::{EventWithInfo, MadaraMaybePendingBlockInfo, MadaraPendingBlockInfo, TransactionWithReceipt};
use mp_class::{ClassInfo, CompiledSierra};
use mp_convert::Felt;
use std::{
    ops::{Bound, RangeBounds},
    sync::Arc,
};

impl<D: MadaraStorageRead> MadaraBackendView<D> {
    fn lookup_preconfirmed_state<V>(
        &self,
        f: impl FnMut((usize, &PreconfirmedBlockTransaction)) -> Option<V>,
    ) -> Option<V> {
        if let Some(preconfirmed) = self.anchor.preconfirmed() {
            preconfirmed.block.content().transactions().iter().enumerate().rev().find_map(f)
        } else {
            None
        }
    }

    pub fn get_contract_storage(&self, contract_address: &Felt, key: &Felt) -> Result<Option<Felt>> {
        let state_diff_key = (*contract_address, *key);
        if let Some(res) = self.lookup_preconfirmed_state(|(_, s)| s.state_diff.storage.get(&state_diff_key).copied()) {
            return Ok(Some(res));
        }
        let Some(block_n) = self.anchor.on_block_n() else { return Ok(None) };
        self.backend.db.get_storage_at(block_n, contract_address, key)
    }

    pub fn get_contract_nonce(&self, contract_address: &Felt) -> Result<Option<Felt>> {
        if let Some(res) = self.lookup_preconfirmed_state(|(_, s)| s.state_diff.nonces.get(contract_address).copied()) {
            return Ok(Some(res));
        }
        let Some(block_n) = self.anchor.on_block_n() else { return Ok(None) };
        self.backend.db.get_contract_nonce_at(block_n, contract_address)
    }

    pub fn get_contract_class_hash(&self, contract_address: &Felt) -> Result<Option<Felt>> {
        if let Some(res) = self.lookup_preconfirmed_state(|(_, s)| s.state_diff.nonces.get(contract_address).copied()) {
            return Ok(Some(res));
        }
        let Some(block_n) = self.anchor.on_block_n() else { return Ok(None) };
        self.backend.db.get_contract_class_hash_at(block_n, contract_address)
    }

    pub fn is_contract_deployed(&self, contract_address: &Felt) -> Result<bool> {
        if self
            .lookup_preconfirmed_state(|(_, s)| {
                if s.state_diff.class_hashes.contains_key(&contract_address) {
                    Some(())
                } else {
                    None
                }
            })
            .is_some()
        {
            return Ok(true);
        }
        let Some(block_n) = self.anchor.on_block_n() else { return Ok(false) };
        self.backend.db.is_contract_deployed_at(block_n, contract_address)
    }

    pub fn get_class(&self, class_hash: &Felt) -> Result<Option<ClassInfo>> {
        if let Some(res) = self.lookup_preconfirmed_state(|(_, s)| {
            s.declared_class.as_ref().filter(|c| c.class_hash() == class_hash).map(|c| c.info())
        }) {
            return Ok(Some(res));
        }
        let Some(block_n) = self.anchor.on_block_n() else { return Ok(None) };
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
        let Some(block_n) = self.anchor.on_block_n() else { return Ok(None) };
        let Some(class) = self.backend.db.get_class_compiled(compiled_class_hash)? else { return Ok(None) };
        if class.block_n <= block_n {
            Ok(Some(class.compiled_sierra.into()))
        } else {
            Ok(None)
        }
    }

    pub fn get_transaction_by_hash(&self, tx_hash: &Felt) -> Result<Option<(TxIndex, TransactionWithReceipt)>> {
        if let Some(res) = self.anchor.preconfirmed().and_then(|preconfirmed| {
            preconfirmed.content().transactions().iter().enumerate().find_map(|(tx_index, tx)| {
                if tx.receipt.transaction_hash() == tx_hash {
                    Some((
                        TxIndex { block_n: preconfirmed.block.block_n, transaction_index: tx_index as _ },
                        TransactionWithReceipt { transaction: tx.transaction.clone(), receipt: tx.receipt.clone() },
                    ))
                } else {
                    None
                }
            })
        }) {
            return Ok(Some(res));
        }

        let Some(on_block_n) = self.anchor.on_block_n() else { return Ok(None) };
        let Some(found) = self.backend.db.find_transaction_hash(tx_hash)? else {
            return Ok(None);
        };

        if found.block_n > on_block_n {
            return Ok(None);
        }

        let tx = self.backend.db.get_transaction(found.block_n, found.transaction_index)?.context("Transaction should exist")?;

        Ok(Some((found, tx)))
    }

    pub fn find_transaction_hash(&self, tx_hash: &Felt) -> Result<Option<TxIndex>> {
        if let Some(res) = self.anchor.preconfirmed().and_then(|preconfirmed| {
            preconfirmed.content().transactions().iter().enumerate().find_map(|(tx_index, tx)| {
                if tx.receipt.transaction_hash() == tx_hash {
                    Some(TxIndex { block_n: preconfirmed.block.block_n, transaction_index: tx_index as _ })
                } else {
                    None
                }
            })
        }) {
            return Ok(Some(res));
        }

        let Some(on_block_n) = self.anchor.on_block_n() else { return Ok(None) };
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
        if let Some(res) = self.anchor.preconfirmed().filter(|p| p.block.block_n == block_n) {
            return Ok(res.content().transactions().get(tx_index_i).map(|tx| TransactionWithReceipt {
                transaction: tx.transaction.clone(),
                receipt: tx.receipt.clone(),
            }));
        }
        if !self.anchor.on_block_n().is_some_and(|on_block_n| on_block_n >= block_n) {
            return Ok(None);
        }

        self.backend.db.get_transaction(block_n, tx_index)
    }

    pub fn get_events(&self, filter: EventFilter) -> Result<Vec<EventWithInfo>> {
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
        vec![]
    }
}

impl<D: MadaraStorageRead> MadaraBackendBlockView<D> {
    pub fn is_preconfirmed(&self) -> bool {
        self.anchor.preconfirmed().is_some()
    }

    pub fn block_n(&self) -> u64 {
        self.anchor.block_n()
    }

    pub fn is_on_l1(&self) -> bool {
        !view.is_preconfirmed()
            && self.backend.get_l1_last_confirmed_block().is_some_and(|last_on_l1| view.block_n() <= last_on_l1)
    }

    pub fn get_block_info(&self) -> Result<MadaraMaybePendingBlockInfo> {
        match self.anchor {
            BlockAnchor::Preconfirmed(preconfirmed) => {
                Ok(MadaraMaybePendingBlockInfo::Pending(MadaraPendingBlockInfo {
                    header: preconfirmed.block.header.clone(),
                    tx_hashes: preconfirmed.content().iter().map(|c| c.receipt.transaction_hash()).collect(),
                }))
            }
            BlockAnchor::OnBlockN(block_n) => Ok(self
                .backend
                .db
                .get_block_info(block_n)?
                .map(MadaraMaybePendingBlockInfo::NotPending)
                .ok_or_else(|| MadaraStorageError::InconsistentStorage("Block info should exist".into()))),
        }
    }

    pub fn get_transaction(&self, tx_index: u64) -> Result<Option<TransactionWithReceipt>> {
        match self.anchor {
            BlockAnchor::Preconfirmed(preconfirmed) => Ok(preconfirmed.content().get(tx_index as _).cloned()),
            BlockAnchor::OnBlockN(block_n) => Ok(self.backend.db.get_transaction(block_n, tx_index)?),
        }
    }

    pub fn get_block_transactions(&self, bounds: impl RangeBounds<u64>) -> Result<Vec<TransactionWithReceipt>> {
        let (start, end) = (bounds.start_bound().cloned(), bounds.end_bound().cloned());

        let start_tx_index: usize = match start.try_into().map_err(|_| MadaraStorageError::InvalidTxIndex) {
            Bound::Excluded(start) => start.saturating_add(1),
            Bound::Included(start) => start,
            Bound::Unbounded => 0,
        };
        let end_tx_index: usize = match end.try_into().map_err(|_| MadaraStorageError::InvalidTxIndex) {
            Bound::Excluded(end) => end,
            Bound::Included(end) => end.saturating_add(1),
            Bound::Unbounded => usize::MAX.into(),
        };
        let to_take = end_tx_index.saturating_sub(start_tx_index);
        if to_take == 0 {
            return Ok(vec![]);
        }

        if let Some(preconfirmed) = self.anchor.preconfirmed() {}
        let Some(block_n) = self.anchor.on_block_n() else { return Ok(vec![]) };

        match self.anchor {
            BlockAnchor::Preconfirmed(preconfirmed) => Ok(preconfirmed
                .content()
                .iter()
                .skip(start_tx_index)
                .take(end_tx_index)
                .map(|tx| TransactionWithReceipt { transaction: tx.transaction.clone(), receipt: tx.receipt.clone() })
                .collect()),
            BlockAnchor::OnBlockN(block_n) => Ok(self
                .backend
                .db
                .get_block_transactions(block_n, start_tx_index as _)
                .take(to_take)
                .collect::<Result<_, _>>()?),
        }
    }
}
