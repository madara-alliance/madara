use std::sync::Arc;

use starknet_core::types::Felt;

use crate::{
    db_block_id::{DbBlockId, DbBlockIdResolvable},
    storage_handler::{
        contract_data::{ContractClassView, ContractNoncesView},
        contract_storage::ContractStorageView,
        DeoxysStorageError,
    },
    Column, DatabaseExt, DeoxysBackend,
};

// #[derive(Debug)]
// pub struct ContractDb {
//     db: Arc<DB>,
// }

impl DeoxysBackend {
    fn resolve_history_kv<K: serde::Serialize, V: serde::de::DeserializeOwned>(
        &self,
        id: &impl DbBlockIdResolvable,
        pending_col: Column,
        active_f: impl FnOnce(u64) -> Result<Option<V>, DeoxysStorageError>,
        k: &K,
    ) -> Result<Option<V>, DeoxysStorageError> {
        let Some(id) = id.resolve_db_block_id(self)? else { return Ok(None) };

        let block_n = match id {
            DbBlockId::Pending => {
                // Get pending or fallback to latest block_n
                let col = self.db.get_column(pending_col);
                // todo: smallint here to avoid alloc
                if let Some(res) = self.db.get_pinned_cf(&col, bincode::serialize(k)?)? {
                    return Ok(Some(bincode::deserialize_from(&*res)?)); // found in pending
                }

                let Some(block_n) = self.get_latest_block_n()? else { return Ok(None) };
                block_n
            }
            DbBlockId::BlockN(block_n) => block_n,
        };

        // todo avoid double serialization
        active_f(block_n)
    }

    pub fn get_contract_class_hash_at(
        &self,
        id: &impl DbBlockIdResolvable,
        contract: &Felt,
    ) -> Result<Option<Felt>, DeoxysStorageError> {
        self.resolve_history_kv(
            id,
            Column::PendingContractToClassHashes,
            |block_n| ContractClassView::new(Arc::clone(&self.db)).get_at(contract, block_n),
            contract,
        )
    }

    pub fn get_contract_nonce_at(
        &self,
        id: &impl DbBlockIdResolvable,
        contract: &Felt,
    ) -> Result<Option<Felt>, DeoxysStorageError> {
        self.resolve_history_kv(
            id,
            Column::PendingContractToNonces,
            |block_n| ContractNoncesView::new(Arc::clone(&self.db)).get_at(contract, block_n),
            contract,
        )
    }

    pub fn get_contract_storage_at(
        &self,
        id: &impl DbBlockIdResolvable,
        contract: &Felt,
        key: &Felt,
    ) -> Result<Option<Felt>, DeoxysStorageError> {
        self.resolve_history_kv(
            id,
            Column::PendingContractStorage,
            |block_n| ContractStorageView::new(Arc::clone(&self.db)).get_at(&(*contract, *key), block_n),
            &(*contract, *key),
        )
    }
}
