use std::sync::Arc;

use parity_scale_codec::Encode;
use starknet_api::hash::StarkFelt;
use starknet_api::transaction::Fee;

use crate::{Column, DbError, DB, DatabaseExt};

pub struct L1HandlerTxFeeDb {
    pub(crate) db: Arc<DB>,
}

impl L1HandlerTxFeeDb {
    pub(crate) fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Store the fee paid on l1 for a specific L1Handler transaction
    pub fn store_fee_paid_for_l1_handler_tx(&self, tx_hash: StarkFelt, fee: Fee) -> Result<(), DbError> {
        let column = self.db.get_column(Column::L1HandlerPaidFee);

        self.db.put_cf(&column, &tx_hash.encode(), &fee.0.to_le_bytes())?;
        Ok(())
    }

    /// Return the stored fee paid on l1 for a specific L1Handler transaction
    pub fn get_fee_paid_for_l1_handler_tx(&self, tx_hash: StarkFelt) -> Result<Fee, DbError> {
        let column = self.db.get_column(Column::L1HandlerPaidFee);

        if let Some(bytes) = self.db.get_cf(&column, &tx_hash.encode())? {
            let mut buff = [0u8; 16];

            buff.copy_from_slice(&bytes);
            let fee = u128::from_le_bytes(buff);

            Ok(Fee(fee))
        } else {
            Err(DbError::ValueNotInitialized(Column::L1HandlerPaidFee, tx_hash.to_string()))
        }
    }
}
