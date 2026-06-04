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
//! The module is structured around four core traits:
//!
//! - [`SubmitTransaction`]: Public interface for submitting user transactions with full validation
//! - [`SubmitL1HandlerTransaction`]: Specialized interface for L1-originated transactions
//! - [`SubmitValidatedTransaction`]: Internal interface for pre-validated transactions
//! - [`TransactionLookup`]: Read-side transaction monitoring and feeder-compatible lookups
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
//! Implementations can provide transaction monitoring through [`TransactionLookup`]:
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
//! [`TransactionLookup`]: crate::TransactionLookup
//! [`TransactionValidator`]: crate::TransactionValidator
//! [`TransactionValidatorConfig`]: crate::TransactionValidatorConfig
//! [`SubmitTransactionError`]: crate::SubmitTransactionError
//! [`RejectedTransactionError`]: crate::RejectedTransactionError
//! [`RejectedTransactionErrorKind`]: crate::RejectedTransactionErrorKind
//! [`StatefulValidator`]: blockifier::blockifier::stateful_validator::StatefulValidator
use async_trait::async_trait;
use mc_db::{view::ExecutedTransactionWithBlockView, MadaraStorage, MadaraStorageRead};
use mc_mempool::{Mempool, PreConfirmationStatus, TransactionStatus as MempoolTransactionStatus};
use mp_gateway::{
    feeder::{ProviderTransactionResponse, ProviderTransactionStatus, TransactionExecutionStatus, TransactionStatus},
    transaction::Transaction as GatewayTransaction,
};
use mp_rpc::admin::BroadcastedDeclareTxnV0;
use mp_rpc::v0_10_2::BroadcastedInvokeTxn;
use mp_rpc::v0_9_0::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, ClassAndTxnHash, ContractAndTxnHash,
};
use mp_transactions::{
    validated::ValidatedTransaction, L1HandlerTransactionResult, L1HandlerTransactionWithFee, TransactionWithHash,
};
mod error;
mod validation;

pub use error::*;
pub use validation::{TransactionValidator, TransactionValidatorConfig};

fn gateway_executed_transaction(transaction: &mp_block::TransactionWithReceipt) -> GatewayTransaction {
    GatewayTransaction::new(
        TransactionWithHash {
            transaction: transaction.transaction.clone(),
            hash: *transaction.receipt.transaction_hash(),
        },
        transaction.receipt.contract_address().copied(),
    )
}

fn execution_status(receipt: &mp_receipt::TransactionReceipt) -> (TransactionExecutionStatus, Option<String>) {
    match receipt.execution_result() {
        mp_receipt::ExecutionResult::Succeeded => (TransactionExecutionStatus::Succeeded, None),
        mp_receipt::ExecutionResult::Reverted { reason } => {
            (TransactionExecutionStatus::Reverted, Some(reason.clone()))
        }
    }
}

fn accepted_status(is_on_l1: bool) -> TransactionStatus {
    if is_on_l1 {
        TransactionStatus::AcceptedOnL1
    } else {
        TransactionStatus::AcceptedOnL2
    }
}

fn transaction_status_from_block<D: MadaraStorageRead>(
    block: &mc_db::MadaraBlockView<D>,
) -> anyhow::Result<(TransactionStatus, Option<mp_convert::Felt>, Option<u64>)> {
    let block_info = block.get_block_info()?;
    if block.as_preconfirmed().is_some() {
        Ok((TransactionStatus::Pending, None, Some(block_info.block_number())))
    } else {
        Ok((accepted_status(block.is_on_l1()), block_info.block_hash().copied(), Some(block_info.block_number())))
    }
}

pub fn feeder_status_from_backend_view<D: MadaraStorageRead>(
    executed: &ExecutedTransactionWithBlockView<D>,
) -> anyhow::Result<ProviderTransactionStatus> {
    if executed.block.as_preconfirmed().is_some() {
        return Ok(ProviderTransactionStatus::not_received());
    }

    let transaction = executed.get_transaction()?;
    let (status, block_hash, _) = transaction_status_from_block(&executed.block)?;
    let (execution_status, tx_revert_reason) = execution_status(&transaction.receipt);

    Ok(ProviderTransactionStatus::with_status(status, Some(execution_status), block_hash, tx_revert_reason))
}

fn feeder_status_from_confirmed_mempool<D: MadaraStorageRead>(
    transaction_hash: mp_convert::Felt,
    mempool: &Mempool<D>,
) -> anyhow::Result<ProviderTransactionStatus> {
    let Some(executed) = mempool.find_transaction_by_hash(&transaction_hash)? else {
        return Ok(ProviderTransactionStatus::not_received());
    };
    feeder_status_from_backend_view(&executed)
}

pub fn feeder_transaction_from_backend_view<D: MadaraStorageRead>(
    executed: &ExecutedTransactionWithBlockView<D>,
) -> anyhow::Result<ProviderTransactionResponse> {
    if executed.block.as_preconfirmed().is_some() {
        return Ok(ProviderTransactionResponse::not_received());
    }

    let transaction = executed.get_transaction()?;
    let (status, block_hash, block_number) = transaction_status_from_block(&executed.block)?;
    let (execution_status, _) = execution_status(&transaction.receipt);

    Ok(ProviderTransactionResponse::with_status(
        status,
        Some(execution_status),
        block_hash,
        block_number,
        Some(executed.transaction_index),
        Some(gateway_executed_transaction(&transaction)),
    ))
}

fn feeder_transaction_from_confirmed_mempool<D: MadaraStorageRead>(
    transaction_hash: mp_convert::Felt,
    mempool: &Mempool<D>,
) -> anyhow::Result<ProviderTransactionResponse> {
    let Some(executed) = mempool.find_transaction_by_hash(&transaction_hash)? else {
        return Ok(ProviderTransactionResponse::not_received());
    };

    feeder_transaction_from_backend_view(&executed)
}

// Feeder `get_transaction` and `get_transaction_status` intentionally hide all preconfirmed
// states. Use `get_preconfirmed_block` or RPC for candidate/preconfirmed transaction data.
fn feeder_status_from_preconfirmed_mempool(_status: &PreConfirmationStatus) -> ProviderTransactionStatus {
    ProviderTransactionStatus::not_received()
}

fn feeder_transaction_from_preconfirmed_mempool(_status: &PreConfirmationStatus) -> ProviderTransactionResponse {
    ProviderTransactionResponse::not_received()
}

/// Read-side transaction monitoring and feeder-compatible lookup surface.
#[async_trait]
pub trait TransactionLookup: Send + Sync {
    async fn received_transaction(&self, hash: mp_convert::Felt) -> Option<bool>;

    async fn subscribe_new_transactions(&self) -> Option<tokio::sync::broadcast::Receiver<mp_convert::Felt>>;

    /// Returns the exact feeder gateway transaction status shape when the backend can provide it.
    ///
    /// `Ok(None)` means the backend does not provide richer feeder semantics and callers should
    /// fall back to local handling. Errors should be propagated so gateway clients do not silently
    /// downgrade transport or upstream failures into `NOT_RECEIVED`.
    async fn feeder_transaction_status(
        &self,
        hash: mp_convert::Felt,
    ) -> Result<Option<ProviderTransactionStatus>, SubmitTransactionError> {
        Ok(self.received_transaction(hash).await.map(|_| ProviderTransactionStatus::not_received()))
    }

    /// Returns the exact feeder gateway transaction payload when the backend can provide it.
    ///
    /// `Ok(None)` means the backend does not provide richer feeder semantics and callers should
    /// fall back to local handling. Errors should be propagated so gateway clients do not silently
    /// downgrade transport or upstream failures into `NOT_RECEIVED`.
    async fn feeder_transaction(
        &self,
        hash: mp_convert::Felt,
    ) -> Result<Option<ProviderTransactionResponse>, SubmitTransactionError> {
        Ok(self.received_transaction(hash).await.map(|_| ProviderTransactionResponse::not_received()))
    }
}

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
}

pub trait ValidatedTransactionProvider: SubmitValidatedTransaction + TransactionLookup {}

impl<T> ValidatedTransactionProvider for T where T: SubmitValidatedTransaction + TransactionLookup + ?Sized {}

#[async_trait]
impl<D: MadaraStorage> TransactionLookup for Mempool<D> {
    async fn received_transaction(&self, hash: mp_convert::Felt) -> Option<bool> {
        Some(self.is_transaction_in_mempool(&hash))
    }
    async fn subscribe_new_transactions(&self) -> Option<tokio::sync::broadcast::Receiver<mp_convert::Felt>> {
        None
    }

    async fn feeder_transaction_status(
        &self,
        hash: mp_convert::Felt,
    ) -> Result<Option<ProviderTransactionStatus>, SubmitTransactionError> {
        match self.get_transaction_status(&hash) {
            Ok(Some(MempoolTransactionStatus::Preconfirmed(status))) => {
                Ok(Some(feeder_status_from_preconfirmed_mempool(&status)))
            }
            Ok(Some(MempoolTransactionStatus::Confirmed { .. })) => {
                Ok(Some(feeder_status_from_confirmed_mempool(hash, self)?))
            }
            Ok(None) => Ok(Some(ProviderTransactionStatus::not_received())),
            Err(err) => Err(SubmitTransactionError::Internal(err)),
        }
    }

    async fn feeder_transaction(
        &self,
        hash: mp_convert::Felt,
    ) -> Result<Option<ProviderTransactionResponse>, SubmitTransactionError> {
        match self.get_transaction_status(&hash) {
            Ok(Some(MempoolTransactionStatus::Preconfirmed(status))) => {
                Ok(Some(feeder_transaction_from_preconfirmed_mempool(&status)))
            }
            Ok(Some(MempoolTransactionStatus::Confirmed { .. })) => {
                Ok(Some(feeder_transaction_from_confirmed_mempool(hash, self)?))
            }
            Ok(None) => Ok(Some(ProviderTransactionResponse::not_received())),
            Err(err) => Err(SubmitTransactionError::Internal(err)),
        }
    }
}

#[async_trait]
impl<D: MadaraStorage> SubmitValidatedTransaction for Mempool<D> {
    async fn submit_validated_transaction(&self, tx: ValidatedTransaction) -> Result<(), SubmitTransactionError> {
        Ok(self.accept_tx(tx).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mc_db::preconfirmed::PreconfirmedBlock;
    use mc_db::MadaraBackend;
    use mc_mempool::MempoolConfig;
    use mp_block::TransactionWithReceipt;
    use mp_receipt::{ExecutionResult, InvokeTransactionReceipt, TransactionReceipt};
    use mp_rpc::v0_9_0::{BroadcastedInvokeTxn, DaMode, InvokeTxnV3, ResourceBounds, ResourceBoundsMapping};
    use starknet_types_core::felt::Felt;
    use std::sync::Arc;

    fn validated_invoke_tx(hash: Felt) -> ValidatedTransaction {
        let tx = BroadcastedInvokeTxn::V3(InvokeTxnV3 {
            calldata: Default::default(),
            sender_address: Felt::ONE,
            signature: Default::default(),
            nonce: Default::default(),
            resource_bounds: ResourceBoundsMapping {
                l1_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
                l2_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
                l1_data_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
            },
            tip: Default::default(),
            paymaster_data: Default::default(),
            account_deployment_data: Default::default(),
            nonce_data_availability_mode: DaMode::L1,
            fee_data_availability_mode: DaMode::L1,
        });

        ValidatedTransaction {
            transaction: mp_transactions::Transaction::Invoke(tx.into()),
            paid_fee_on_l1: None,
            contract_address: Felt::ONE,
            arrived_at: mp_transactions::validated::TxTimestamp::now(),
            declared_class: None,
            hash,
            charge_fee: true,
        }
    }

    fn executed_preconfirmed_tx(
        hash: Felt,
        execution_result: ExecutionResult,
    ) -> mc_db::preconfirmed::PreconfirmedExecutedTransaction {
        mc_db::preconfirmed::PreconfirmedExecutedTransaction {
            transaction: TransactionWithReceipt {
                transaction: validated_invoke_tx(hash).transaction,
                receipt: TransactionReceipt::Invoke(InvokeTransactionReceipt {
                    transaction_hash: hash,
                    execution_result,
                    ..Default::default()
                }),
            },
            state_diff: Default::default(),
            declared_class: None,
            arrived_at: mp_transactions::validated::TxTimestamp::now(),
            paid_fee_on_l1: None,
        }
    }

    fn backend_for_tests() -> Arc<MadaraBackend> {
        let chain_config = Arc::new(mp_chain_config::ChainConfig::madara_test());
        mc_db::MadaraBackend::open_for_testing(chain_config)
    }

    fn confirmed_block(hash: Felt) -> mp_block::FullBlockWithoutCommitments {
        mp_block::FullBlockWithoutCommitments {
            header: mp_block::header::PreconfirmedHeader { block_number: 0, ..Default::default() },
            state_diff: Default::default(),
            transactions: vec![TransactionWithReceipt {
                transaction: validated_invoke_tx(hash).transaction,
                receipt: TransactionReceipt::Invoke(InvokeTransactionReceipt {
                    transaction_hash: hash,
                    execution_result: ExecutionResult::Succeeded,
                    ..Default::default()
                }),
            }],
            events: Default::default(),
        }
    }

    #[test]
    fn feeder_transaction_from_mempool_received_is_not_received() {
        let tx = Arc::new(validated_invoke_tx(Felt::TWO));
        let response = feeder_transaction_from_preconfirmed_mempool(&PreConfirmationStatus::Received(Arc::clone(&tx)));

        assert_eq!(response, ProviderTransactionResponse::not_received());
    }

    #[test]
    fn feeder_transaction_from_mempool_candidate_is_not_received() {
        let tx = Arc::new(validated_invoke_tx(Felt::THREE));
        let view = Arc::new(PreconfirmedBlock::new(Default::default()));
        let response = feeder_transaction_from_preconfirmed_mempool(&PreConfirmationStatus::Candidate {
            view,
            transaction_index: 3,
            transaction: Arc::clone(&tx),
        });

        assert_eq!(response, ProviderTransactionResponse::not_received());
    }

    #[test]
    fn feeder_transaction_from_mempool_executed_is_not_received() {
        let executed_hash = Felt::from_hex_unchecked("0x4");
        let tx = Arc::new(executed_preconfirmed_tx(executed_hash, ExecutionResult::Succeeded));
        let view = Arc::new(PreconfirmedBlock::new_with_content(Default::default(), [tx.as_ref().clone()], []));
        let response = feeder_transaction_from_preconfirmed_mempool(&PreConfirmationStatus::Executed {
            view,
            transaction_index: 0,
        });

        assert_eq!(response, ProviderTransactionResponse::not_received());
    }

    #[test]
    fn feeder_transaction_status_from_mempool_executed_is_not_received() {
        let tx = Arc::new(executed_preconfirmed_tx(
            Felt::from_hex_unchecked("0x5"),
            ExecutionResult::Reverted { reason: "boom".into() },
        ));
        let status = feeder_status_from_preconfirmed_mempool(&PreConfirmationStatus::Executed {
            view: Arc::new(PreconfirmedBlock::new_with_content(Default::default(), [tx.as_ref().clone()], [])),
            transaction_index: 0,
        });

        assert_eq!(status, ProviderTransactionStatus::not_received());
    }

    #[tokio::test]
    async fn mempool_confirmed_status_includes_execution_status_and_block_hash() {
        let hash = Felt::from_hex_unchecked("0x6");
        let backend = backend_for_tests();
        backend
            .write_access()
            .add_full_block_with_classes(&confirmed_block(hash), &[], true)
            .expect("Failed to persist confirmed block");
        let mempool = Mempool::new(backend, MempoolConfig::default());

        let status = TransactionLookup::feeder_transaction_status(&mempool, hash).await.unwrap().unwrap();

        assert_eq!(status.tx_status, TransactionStatus::AcceptedOnL2);
        assert_eq!(status.finality_status, TransactionStatus::AcceptedOnL2);
        assert_eq!(status.execution_status, Some(TransactionExecutionStatus::Succeeded));
        assert!(status.block_hash.is_some());
    }

    #[tokio::test]
    async fn mempool_confirmed_transaction_includes_block_hash_and_payload() {
        let hash = Felt::from_hex_unchecked("0x7");
        let backend = backend_for_tests();
        backend
            .write_access()
            .add_full_block_with_classes(&confirmed_block(hash), &[], true)
            .expect("Failed to persist confirmed block");
        let mempool = Mempool::new(backend, MempoolConfig::default());

        let response = TransactionLookup::feeder_transaction(&mempool, hash).await.unwrap().unwrap();

        assert_eq!(response.status, TransactionStatus::AcceptedOnL2);
        assert_eq!(response.finality_status, TransactionStatus::AcceptedOnL2);
        assert_eq!(response.execution_status, Some(TransactionExecutionStatus::Succeeded));
        assert!(response.block_hash.is_some());
        assert_eq!(response.block_number, Some(0));
        assert_eq!(response.transaction_index, Some(0));
        assert_eq!(response.transaction.as_ref().map(|tx| *tx.transaction_hash()), Some(hash));
    }

    #[tokio::test]
    async fn mempool_confirmed_status_without_backend_record_is_not_received() {
        let hash = Felt::from_hex_unchecked("0x8");
        let backend = backend_for_tests();
        let mempool = Mempool::new(backend, MempoolConfig::default());

        let status = feeder_status_from_confirmed_mempool(hash, &mempool).unwrap();

        assert_eq!(status, ProviderTransactionStatus::not_received());
    }

    #[tokio::test]
    async fn mempool_confirmed_transaction_without_backend_record_is_not_received() {
        let hash = Felt::from_hex_unchecked("0x9");
        let backend = backend_for_tests();
        let mempool = Mempool::new(backend, MempoolConfig::default());

        let response = feeder_transaction_from_confirmed_mempool(hash, &mempool).unwrap();

        assert_eq!(response, ProviderTransactionResponse::not_received());
    }
}
