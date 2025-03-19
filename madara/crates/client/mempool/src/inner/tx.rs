use blockifier::transaction::transaction_execution::Transaction;
use mc_exec::execution::TxInfo;
use mp_class::ConvertedClass;
use mp_convert::FeltHexDisplay;
use starknet_api::{
    core::{ContractAddress, Nonce},
    transaction::TransactionHash,
    StarknetApiError,
};
use std::{fmt, time::SystemTime};

pub type ArrivedAtTimestamp = SystemTime;

/// Wrapper around a blockifier [Transaction] with some added information needed
/// by the [Mempool]
///
/// [Mempool]: super::super::Mempool
#[derive(Clone)]
pub struct MempoolTransaction {
    pub tx: Transaction,
    /// Time at which the transaction was inserted into the mempool (+ or -)
    pub arrived_at: ArrivedAtTimestamp,
    /// TODO: What is this?
    pub converted_class: Option<ConvertedClass>,
    /// We need this to be able to retrieve the transaction once from the
    /// [NonceTxMapping] once it has been inserted into the [Mempool]
    ///
    /// [NonceTxMapping]: super::NonceTxMapping
    /// [Mempool]: super::super::Mempool
    pub nonce: Nonce,
    /// We include this in the struct to avoid having to recompute the next
    /// nonce of a ready transaction once it is re-added into the [Mempool] by
    /// the block production task.
    ///
    /// [Mempool]: super::super::Mempool
    pub nonce_next: Nonce,
}

impl fmt::Debug for MempoolTransaction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MempoolTransaction")
            .field("tx_hash", &self.tx_hash().hex_display())
            .field("nonce", &self.nonce.hex_display())
            .field("nonce_next", &self.nonce_next.hex_display())
            .field("contract_address", &self.contract_address().hex_display())
            .field("tx_type", &self.tx.tx_type())
            .field("arrived_at", &self.arrived_at)
            .finish()
    }
}

impl MempoolTransaction {
    pub fn new_from_blockifier_tx(
        tx: Transaction,
        arrived_at: ArrivedAtTimestamp,
        converted_class: Option<ConvertedClass>,
    ) -> Result<Self, StarknetApiError> {
        let nonce = tx.nonce();
        let nonce_next = nonce.try_increment()?;

        Ok(Self { tx, arrived_at, converted_class, nonce, nonce_next })
    }
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
