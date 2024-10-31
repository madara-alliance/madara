use crate::{clone_transaction, contract_addr, nonce, tx_hash};
use blockifier::transaction::transaction_execution::Transaction;
use mc_exec::execution::TxInfo;
use mp_class::ConvertedClass;
use mp_convert::FeltHexDisplay;
use starknet_api::{
    core::{ContractAddress, Nonce},
    transaction::TransactionHash,
};
use std::{fmt, time::SystemTime};

pub type ArrivedAtTimestamp = SystemTime;

pub struct MempoolTransaction {
    pub tx: Transaction,
    pub arrived_at: ArrivedAtTimestamp,
    pub converted_class: Option<ConvertedClass>,
}

impl fmt::Debug for MempoolTransaction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MempoolTransaction")
            .field("tx_hash", &self.tx_hash().hex_display())
            .field("nonce", &self.nonce().hex_display())
            .field("contract_address", &self.contract_address().hex_display())
            .field("tx_type", &self.tx.tx_type())
            .field("arrived_at", &self.arrived_at)
            .finish()
    }
}

impl Clone for MempoolTransaction {
    fn clone(&self) -> Self {
        Self {
            tx: clone_transaction(&self.tx),
            arrived_at: self.arrived_at,
            converted_class: self.converted_class.clone(),
        }
    }
}

impl MempoolTransaction {
    pub fn clone_tx(&self) -> Transaction {
        clone_transaction(&self.tx)
    }
    pub fn nonce(&self) -> Nonce {
        nonce(&self.tx)
    }
    pub fn contract_address(&self) -> ContractAddress {
        contract_addr(&self.tx)
    }
    pub fn tx_hash(&self) -> TransactionHash {
        tx_hash(&self.tx)
    }
}
