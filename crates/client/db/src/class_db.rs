use std::sync::Arc;

use dp_class::ContractClassData;
use starknet_core::types::Felt;

use crate::{
    db_block_id::{DbBlockId, DbBlockIdResolvable},
    storage_handler::{
        contract_class_data::ContractClassDataView, contract_class_hashes::ContractClassHashesView, DeoxysStorageError,
        StorageView,
    },
    DeoxysBackend,
};

impl DeoxysBackend {
    pub fn get_class_info(
        &self,
        id: &impl DbBlockIdResolvable,
        class: &Felt,
    ) -> Result<Option<ContractClassData>, DeoxysStorageError> {
        let Some(id) = id.resolve_db_block_id(self)? else { return Ok(None) };

        let Some(info) = ContractClassDataView::new(Arc::clone(&self.db)).get(class)? else { return Ok(None) };

        let valid = match (id, info.block_number) {
            (DbBlockId::Pending, None) => true,
            (DbBlockId::BlockN(block_n), Some(real_block_n)) => real_block_n >= block_n,
            _ => false,
        };
        if !valid {
            return Ok(None);
        }

        Ok(Some(info))
    }

    pub fn get_compiled_class(
        &self,
        id: &impl DbBlockIdResolvable,
        class: &Felt,
    ) -> Result<Option<(ContractClassData, Felt)>, DeoxysStorageError> {
        let Some(info) = self.get_class_info(id, class)? else { return Ok(None) };
        let Some(compiled) = ContractClassHashesView::new(Arc::clone(&self.db)).get(class)? else { return Ok(None) };
        Ok(Some((info, compiled)))
    }
}
