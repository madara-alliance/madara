use async_trait::async_trait;
use mc_db::MadaraStorage;
use mc_mempool::Mempool;
use mp_rpc::admin::BroadcastedDeclareTxnV0;
use mp_rpc::v0_7_1::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn,
    ClassAndTxnHash, ContractAndTxnHash,
};
use mp_transactions::validated::ValidatedTransaction;

mod error;
mod validation;

pub use error::*;
pub use validation::{TransactionValidator, TransactionValidatorConfig};

/// Abstraction layer over where transactions are submitted.
///
/// This is usually implemented by the local-run mempool or a client to another node's gateway interface,
/// and is usuallt used by the RPC, gateway and p2p interfaces.
#[async_trait]
pub trait SubmitTransaction: Send + Sync {
    /// Madara specific.
    async fn submit_declare_v0_transaction(
        &self,
        _tx: BroadcastedDeclareTxnV0,
    ) -> Result<ClassAndTxnHash, SubmitTransactionError> {
        Err(SubmitTransactionError::Unsupported)
    }

    async fn submit_declare_transaction(
        &self,
        tx: BroadcastedDeclareTxn,
    ) -> Result<ClassAndTxnHash, SubmitTransactionError>;

    async fn submit_deploy_account_transaction(
        &self,
        tx: BroadcastedDeployAccountTxn,
    ) -> Result<ContractAndTxnHash, SubmitTransactionError>;

    async fn submit_invoke_transaction(
        &self,
        tx: BroadcastedInvokeTxn,
    ) -> Result<AddInvokeTransactionResult, SubmitTransactionError>;

    async fn received_transaction(&self, hash: mp_convert::Felt) -> Option<bool>;

    async fn subscribe_new_transactions(&self) -> Option<tokio::sync::broadcast::Receiver<mp_convert::Felt>>;
}

/// Submit a validated transaction. Note: No validation will be performed on the transaction.
/// This should never be directly exposed to users.
#[async_trait]
pub trait SubmitValidatedTransaction: Send + Sync {
    async fn submit_validated_transaction(&self, tx: ValidatedTransaction) -> Result<(), SubmitTransactionError>;

    async fn received_transaction(&self, hash: mp_convert::Felt) -> Option<bool>;

    async fn subscribe_new_transactions(&self) -> Option<tokio::sync::broadcast::Receiver<mp_convert::Felt>>;
}

#[async_trait]
impl<D: MadaraStorage> SubmitValidatedTransaction for Mempool<D> {
    async fn submit_validated_transaction(&self, tx: ValidatedTransaction) -> Result<(), SubmitTransactionError> {
        Ok(self.accept_tx(tx).await?)
    }
    async fn received_transaction(&self, hash: mp_convert::Felt) -> Option<bool> {
        Some(self.is_transaction_in_mempool(&hash))
    }
    async fn subscribe_new_transactions(&self) -> Option<tokio::sync::broadcast::Receiver<mp_convert::Felt>> {
        None
    }
}
