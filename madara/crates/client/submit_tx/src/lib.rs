//! Madara transaction submission layer. This crate provides an abstraction over where transactions
//! are submitted, typically to the local mempool or another node's gateway interface.
//!
//! # Overview
//!
//! The submit-tx module acts as the primary entry point for all transaction submissions in Madara.
//! It provides a unified interface for submitting transactions from various sources (RPC, gateway,
//! P2P) while handling validation, error mapping, and routing to the appropriate backend.
//!
//! This abstraction allows Madara to seamlessly switch between different submission targets: local
//! mempool for sequencer nodes, remote gateways for full nodes, or even a test implementations for
//! development.
//!
//! # Architecture
//!
//! The module is structured around three core traits that define different submission pathways:
//!
//! - [`SubmitTransaction`]: Public interface for submitting user transactions with full validation
//! - [`SubmitL1HandlerTransaction`]: Specialized interface for L1-originated transactions
//! - [`SubmitValidatedTransaction`]: Internal interface for pre-validated transactions
//!
//! # Transaction Validation
//!
//! The [`TransactionValidator`] wraps any [`SubmitValidatedTransaction`] implementation and is
//! responsible for validating transactions before forwarding them.
//!
//! ## Validation Checks
//!
//! A [`TransactionValidator`] performs the following checks:
//!
//! - Version checks (rejects v0 transactions).
//! - Query-only transaction rejection.
//! - Nonce verification against current account state.
//! - Checks for sufficient fees for execution (unless disabled).
//!
//! # Transaction Submission Flow
//!
//! When a transaction is submitted through the RPC or gateway:
//!
//! 1. **Transaction arrives** via one of the submission methods (invoke, declare, deploy_account)
//! 2. **Format conversion**: The transaction is converted from RPC format to Starknet API format
//! 3. **Pre-validation checks**:
//!    - Query-only transactions are rejected immediately
//!    - Version compatibility is verified
//!    - Transaction type-specific checks are performed
//! 4. **Stateful validation** (if enabled):
//!    - A [`StatefulValidator`] is created from the current blockchain state
//!    - Account nonce is verified
//!    - Fees are checked (unless disabled or for admin declare v0)
//! 5. **Forwarding**: The validated transaction is forwarded to the configured backend or remote
//!    node gateway
//! 6. **Response generation**: Transaction hash and relevant data are returned to the caller
//!
//! # Configuration
//!
//! Transaction validation behavior can be customized through [`TransactionValidatorConfig`]:
//!
//! ```no_run
//! let config = TransactionValidatorConfig::default()
//!     .with_disable_validation(true);  // Skip validation for testing
//!
//! let validator = TransactionValidator::new(
//!     backend_impl,
//!     madara_backend,
//!     config
//! );
//! ```
//!
//! ## Configuration Options
//!
//! - `disable_validation`: Skip all validation checks (dangerous, testing only)
//! - `disable_fee`: Skip fee-related checks (useful for development networks)
//!
//! # Special Transaction Types
//!
//! ## Admin Declare V0
//!
//! Madara supports legacy Declare V0 transactions through a special admin endpoint. These
//! transactions bypass fee validation as they predate the fee mechanism.
//!
//! ## L1 Handler Transactions
//!
//! L1-originated transactions follow a separate path through [`SubmitL1HandlerTransaction`].
//! These transactions don't have nonces and include L1 fee payment information.
//!
//! ## Deploy Account Transactions
//!
//! Deploy account transactions receive special handling: invoke transactions with nonce 1
//! from the same account skip certain validations since the account doesn't exist yet.
//!
//! # Transaction Monitoring
//!
//! Implementations can provide transaction monitoring through:
//!
//! - `received_transaction`: Check if a transaction hash exists
//! - `subscribe_new_transactions`: Real-time updates via broadcast channel
//!
//! These methods return an [`Option`] to indicate whether the backend supports monitoring. This is
//! used for example by `mc-mempool` to stream the status of its transactions as it receives them.
//!
//! [`SubmitTransaction`]: crate::SubmitTransaction
//! [`SubmitL1HandlerTransaction`]: crate::SubmitL1HandlerTransaction
//! [`SubmitValidatedTransaction`]: crate::SubmitValidatedTransaction
//! [`TransactionValidator`]: crate::TransactionValidator
//! [`TransactionValidatorConfig`]: crate::TransactionValidatorConfig
//! [`SubmitTransactionError`]: crate::SubmitTransactionError
//! [`RejectedTransactionError`]: crate::RejectedTransactionError
//! [`RejectedTransactionErrorKind`]: crate::RejectedTransactionErrorKind
//! [`StatefulValidator`]: blockifier::blockifier::stateful_validator::StatefulValidator
use async_trait::async_trait;
use mc_db::MadaraStorage;
use mc_mempool::Mempool;
use mp_rpc::admin::BroadcastedDeclareTxnV0;
use mp_rpc::v0_9_0::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn,
    ClassAndTxnHash, ContractAndTxnHash,
};
use mp_transactions::{validated::ValidatedTransaction, L1HandlerTransactionResult, L1HandlerTransactionWithFee};

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

/// Submit a L1HandlerTransaction.
#[async_trait]
pub trait SubmitL1HandlerTransaction: Send + Sync {
    async fn submit_l1_handler_transaction(
        &self,
        tx: L1HandlerTransactionWithFee,
    ) -> Result<L1HandlerTransactionResult, SubmitTransactionError>;
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
