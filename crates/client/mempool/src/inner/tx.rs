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

#[derive(Clone)]
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

impl MempoolTransaction {
    pub fn nonce(&self) -> Nonce {
        self.tx.nonce()
    }
    pub fn contract_address(&self) -> ContractAddress {
        self.tx.sender_address()
    }
    pub fn tx_hash(&self) -> TransactionHash {
        self.tx.tx_hash()
    }
}
