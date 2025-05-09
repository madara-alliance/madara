use async_trait::async_trait;
use mp_rpc::{
    admin::BroadcastedDeclareTxnV0, AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn,
    BroadcastedInvokeTxn, ClassAndTxnHash, ContractAndTxnHash,
};
use mp_transactions::{validated::ValidatedMempoolTx, L1HandlerTransaction, L1HandlerTransactionResult};

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
}

/// Submit an L1HandlerTransaction.
#[async_trait]
pub trait SubmitL1HandlerTransaction: Send + Sync {
    async fn submit_l1_handler_transaction(
        &self,
        tx: L1HandlerTransaction,
        paid_fees_on_l1: u128,
    ) -> Result<L1HandlerTransactionResult, SubmitTransactionError>;
}

/// Submit a validated transaction. Note: No validation will be performed on the transaction.
/// This should never be directly exposed to users.
#[async_trait]
pub trait SubmitValidatedTransaction: Send + Sync {
    async fn submit_validated_transaction(&self, tx: ValidatedMempoolTx) -> Result<(), SubmitTransactionError>;
}
