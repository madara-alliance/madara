use std::sync::Arc;

use parity_scale_codec::{Decode, Encode};
use starknet_api::api_core::ClassHash;
use starknet_api::state::ContractClass;

use crate::{Column, DatabaseExt, DbError, DB};

/// Allow interaction with the sierra classes db
pub struct SierraClassesDb {
    db: Arc<DB>,
}

impl SierraClassesDb {
    pub(crate) fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    pub fn store_sierra_class(&self, class_hash: ClassHash, class: ContractClass) -> Result<(), DbError> {
        let column = self.db.get_column(Column::SierraContractClasses);

        self.db.put_cf(&column, class_hash.encode(), class.encode())?;
        Ok(())
    }

    pub fn get_sierra_class(&self, class_hash: ClassHash) -> Result<Option<ContractClass>, DbError> {
        let column = self.db.get_column(Column::SierraContractClasses);

        let opt_contract_class = self
            .db
            .get_cf(&column, class_hash.encode())?
            .map(|raw| ContractClass::decode(&mut &raw[..]))
            .transpose()?;

        Ok(opt_contract_class)
    }
}
