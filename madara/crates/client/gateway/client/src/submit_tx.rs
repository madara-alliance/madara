use crate::metrics::{
    add_transaction_error_code, add_transaction_error_code_from_submit_error, add_transaction_result,
    add_transaction_result_from_submit_error, add_transaction_tx_type, metrics,
};
use crate::GatewayProvider;
use async_trait::async_trait;
use mc_submit_tx::{RejectedTransactionError, RejectedTransactionErrorKind, SubmitTransaction, SubmitTransactionError};
use mp_gateway::{error::SequencerError, user_transaction::UserTransactionConversionError};
use mp_rpc::v0_10_2::BroadcastedInvokeTxn;
use mp_rpc::v0_9_0::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, ClassAndTxnHash, ContractAndTxnHash,
};
use std::borrow::Cow;
use std::time::{Duration, Instant};

fn rejected(kind: RejectedTransactionErrorKind, message: impl Into<Cow<'static, str>>) -> SubmitTransactionError {
    SubmitTransactionError::Rejected(RejectedTransactionError::new(kind, message))
}

fn map_gateway_error(err: SequencerError) -> SubmitTransactionError {
    use mc_submit_tx::RejectedTransactionErrorKind::*;
    use mp_gateway::error::StarknetErrorCode as GWErrCode;
    use SequencerError as Error;
    use SubmitTransactionError::*;

    // This match is intentionally exhaustive, as to force us to modify it if any of the types change.
    match err {
        Error::NoUrl => Unsupported,
        Error::StarknetError(e) => match e.code {
            GWErrCode::EntryPointNotFound => rejected(EntryPointNotFound, e.message),
            GWErrCode::OutOfRangeContractAddress => rejected(OutOfRangeContractAddress, e.message),
            GWErrCode::TransactionFailed => rejected(TransactionFailed, e.message),
            GWErrCode::UninitializedContract => rejected(UninitializedContract, e.message),
            GWErrCode::OutOfRangeTransactionHash => rejected(OutOfRangeTransactionHash, e.message),
            GWErrCode::UnsupportedSelectorForFee => rejected(UnsupportedSelectorForFee, e.message),
            GWErrCode::InvalidContractDefinition => rejected(InvalidContractDefinition, e.message),
            GWErrCode::NotPermittedContract => rejected(NotPermittedContract, e.message),
            GWErrCode::UndeclaredClass => rejected(UndeclaredClass, e.message),
            GWErrCode::TransactionLimitExceeded => rejected(TransactionLimitExceeded, e.message),
            GWErrCode::InvalidTransactionNonce => rejected(InvalidTransactionNonce, e.message),
            GWErrCode::ReplacementTransactionUnderpriced => rejected(ReplacementTransactionUnderpriced, e.message),
            GWErrCode::FeeBelowMinimum => rejected(FeeBelowMinimum, e.message),
            GWErrCode::OutOfRangeFee => rejected(OutOfRangeFee, e.message),
            GWErrCode::InvalidTransactionVersion => rejected(InvalidTransactionVersion, e.message),
            GWErrCode::InvalidProgram => rejected(InvalidProgram, e.message),
            GWErrCode::DeprecatedTransaction => rejected(DeprecatedTransaction, e.message),
            GWErrCode::InvalidCompiledClassHash => rejected(InvalidCompiledClassHash, e.message),
            GWErrCode::CompilationFailed => rejected(CompilationFailed, e.message),
            GWErrCode::UnauthorizedEntryPointForInvoke => rejected(UnauthorizedEntryPointForInvoke, e.message),
            GWErrCode::InvalidContractClass => rejected(InvalidContractClass, e.message),
            GWErrCode::ClassAlreadyDeclared => rejected(ClassAlreadyDeclared, e.message),
            GWErrCode::InvalidSignature => rejected(InvalidSignature, e.message),
            GWErrCode::InsufficientAccountBalance => rejected(InsufficientAccountBalance, e.message),
            GWErrCode::InsufficientMaxFee => rejected(InsufficientMaxFee, e.message),
            GWErrCode::ValidateFailure => rejected(ValidateFailure, e.message),
            GWErrCode::ContractBytecodeSizeTooLarge => rejected(ContractBytecodeSizeTooLarge, e.message),
            GWErrCode::ContractClassObjectSizeTooLarge => rejected(ContractClassObjectSizeTooLarge, e.message),
            GWErrCode::DuplicatedTransaction => rejected(DuplicatedTransaction, e.message),
            GWErrCode::InvalidContractClassVersion => rejected(InvalidContractClassVersion, e.message),
            GWErrCode::RateLimited => rejected(RateLimited, e.message),

            // These should not really happen?
            GWErrCode::BlockNotFound
            | GWErrCode::NoBlockHeader
            | GWErrCode::SchemaValidationError
            | GWErrCode::OutOfRangeBlockHash
            | GWErrCode::MalformedRequest
            | GWErrCode::NoSignatureForPendingBlock => {
                Internal(anyhow::anyhow!("Gateway returned invalid error code for request: {e:#}"))
            }
        },
        err @ (Error::HyperError(_)
        | Error::InvalidUrl(_)
        | Error::HttpError(_)
        | Error::HttpCallError(_)
        | Error::DeserializeBody { .. }
        | Error::SerializeRequest(_)
        | Error::CompressError(_)
        | Error::InvalidStarknetError { .. }) => Internal(anyhow::anyhow!(err)),
    }
}

fn map_conv_error(error: UserTransactionConversionError) -> SubmitTransactionError {
    use mc_submit_tx::RejectedTransactionErrorKind::*;
    use UserTransactionConversionError as ConvError;

    match error {
        ConvError::UnsupportedQueryTransaction => {
            rejected(InvalidTransactionVersion, "Cannot submit query-only transactions")
        }
        ConvError::ContractClassDecodeError(error) => {
            rejected(InvalidContractClass, format!("Decode error: {error:#}"))
        }
    }
}

fn log_gateway_client_submit_error(tx_type: &'static str, error: &SubmitTransactionError, duration: Duration) {
    let error_code = add_transaction_error_code_from_submit_error(error);
    let result = add_transaction_result_from_submit_error(error);
    let duration_ms = duration.as_secs_f64() * 1000.0;

    match error {
        SubmitTransactionError::Rejected(error) => tracing::warn!(
            target: "gateway_client_transactions",
            service = "gateway",
            endpoint = "add_transaction",
            tx_type,
            result,
            error_code,
            duration_ms,
            error = %error,
            "Gateway client add_transaction rejected"
        ),
        SubmitTransactionError::Internal(error) => tracing::error!(
            target: "gateway_client_transactions",
            service = "gateway",
            endpoint = "add_transaction",
            tx_type,
            result,
            error_code,
            duration_ms,
            error = %error,
            "Gateway client add_transaction failed"
        ),
        SubmitTransactionError::Unsupported => tracing::warn!(
            target: "gateway_client_transactions",
            service = "gateway",
            endpoint = "add_transaction",
            tx_type,
            result,
            error_code,
            duration_ms,
            "Gateway client add_transaction is unsupported"
        ),
    }
}

fn record_gateway_client_submit_success(tx_type: &'static str, duration: Duration) {
    metrics().record_add_transaction(
        tx_type,
        add_transaction_result::SUCCESS,
        add_transaction_error_code::NONE,
        duration,
    );
}

fn record_gateway_client_submit_error(tx_type: &'static str, error: &SubmitTransactionError, duration: Duration) {
    metrics().record_add_transaction(
        tx_type,
        add_transaction_result_from_submit_error(error),
        add_transaction_error_code_from_submit_error(error),
        duration,
    );
    log_gateway_client_submit_error(tx_type, error, duration);
}

#[async_trait]
impl SubmitTransaction for GatewayProvider {
    async fn submit_declare_transaction(
        &self,
        tx: BroadcastedDeclareTxn,
    ) -> Result<ClassAndTxnHash, SubmitTransactionError> {
        let started_at = Instant::now();
        let tx = match tx.try_into().map_err(map_conv_error) {
            Ok(tx) => tx,
            Err(error) => {
                record_gateway_client_submit_error(add_transaction_tx_type::DECLARE, &error, started_at.elapsed());
                return Err(error);
            }
        };

        match self.add_declare_transaction(tx).await {
            Ok(res) => {
                let duration = started_at.elapsed();
                record_gateway_client_submit_success(add_transaction_tx_type::DECLARE, duration);
                tracing::info!(
                    target: "gateway_client_transactions",
                    service = "gateway",
                    endpoint = "add_transaction",
                    tx_type = add_transaction_tx_type::DECLARE,
                    duration_ms = duration.as_secs_f64() * 1000.0,
                    transaction_hash = %format_args!("{:#x}", res.transaction_hash),
                    class_hash = %format_args!("{:#x}", res.class_hash),
                    "Forwarded gateway add_transaction request"
                );
                Ok(ClassAndTxnHash { transaction_hash: res.transaction_hash, class_hash: res.class_hash })
            }
            Err(error) => {
                let error = map_gateway_error(error);
                let duration = started_at.elapsed();
                record_gateway_client_submit_error(add_transaction_tx_type::DECLARE, &error, duration);
                Err(error)
            }
        }
    }

    async fn submit_deploy_account_transaction(
        &self,
        tx: BroadcastedDeployAccountTxn,
    ) -> Result<ContractAndTxnHash, SubmitTransactionError> {
        let started_at = Instant::now();
        let tx = match tx.try_into().map_err(map_conv_error) {
            Ok(tx) => tx,
            Err(error) => {
                record_gateway_client_submit_error(
                    add_transaction_tx_type::DEPLOY_ACCOUNT,
                    &error,
                    started_at.elapsed(),
                );
                return Err(error);
            }
        };

        match self.add_deploy_account_transaction(tx).await {
            Ok(res) => {
                let duration = started_at.elapsed();
                record_gateway_client_submit_success(add_transaction_tx_type::DEPLOY_ACCOUNT, duration);
                tracing::info!(
                    target: "gateway_client_transactions",
                    service = "gateway",
                    endpoint = "add_transaction",
                    tx_type = add_transaction_tx_type::DEPLOY_ACCOUNT,
                    duration_ms = duration.as_secs_f64() * 1000.0,
                    transaction_hash = %format_args!("{:#x}", res.transaction_hash),
                    contract_address = %format_args!("{:#x}", res.address),
                    "Forwarded gateway add_transaction request"
                );
                Ok(ContractAndTxnHash { transaction_hash: res.transaction_hash, contract_address: res.address })
            }
            Err(error) => {
                let error = map_gateway_error(error);
                let duration = started_at.elapsed();
                record_gateway_client_submit_error(add_transaction_tx_type::DEPLOY_ACCOUNT, &error, duration);
                Err(error)
            }
        }
    }

    async fn submit_invoke_transaction(
        &self,
        tx: BroadcastedInvokeTxn,
    ) -> Result<AddInvokeTransactionResult, SubmitTransactionError> {
        let started_at = Instant::now();
        let tx = match tx.try_into().map_err(map_conv_error) {
            Ok(tx) => tx,
            Err(error) => {
                record_gateway_client_submit_error(add_transaction_tx_type::INVOKE, &error, started_at.elapsed());
                return Err(error);
            }
        };

        match self.add_invoke_transaction(tx).await {
            Ok(res) => {
                let duration = started_at.elapsed();
                record_gateway_client_submit_success(add_transaction_tx_type::INVOKE, duration);
                tracing::info!(
                    target: "gateway_client_transactions",
                    service = "gateway",
                    endpoint = "add_transaction",
                    tx_type = add_transaction_tx_type::INVOKE,
                    duration_ms = duration.as_secs_f64() * 1000.0,
                    transaction_hash = %format_args!("{:#x}", res.transaction_hash),
                    "Forwarded gateway add_transaction request"
                );
                Ok(AddInvokeTransactionResult { transaction_hash: res.transaction_hash })
            }
            Err(error) => {
                let error = map_gateway_error(error);
                let duration = started_at.elapsed();
                record_gateway_client_submit_error(add_transaction_tx_type::INVOKE, &error, duration);
                Err(error)
            }
        }
    }

    async fn received_transaction(&self, _hash: starknet_types_core::felt::Felt) -> Option<bool> {
        // The gateway cannot inform us about the status of transactions it has received since this
        // is forwarded to a remote node which does not expose any endpoint to query this state. By
        // default, all transactions which pass through the gateway will be automatically considered
        // as received.
        None
    }

    async fn subscribe_new_transactions(
        &self,
    ) -> Option<tokio::sync::broadcast::Receiver<starknet_types_core::felt::Felt>> {
        // We cannot subscribe to new transactions from the gateway for the same reasons as above
        None
    }
}

#[async_trait]
impl mc_submit_tx::SubmitValidatedTransaction for GatewayProvider {
    async fn submit_validated_transaction(
        &self,
        tx: mp_transactions::validated::ValidatedTransaction,
    ) -> Result<(), SubmitTransactionError> {
        self.add_validated_transaction(tx).await.map_err(map_gateway_error)
    }

    async fn received_transaction(&self, _hash: starknet_types_core::felt::Felt) -> Option<bool> {
        // The gateway cannot inform us about the status of transactions it has received since this
        // is forwarded to a remote node which does not expose any endpoint to query this state. By
        // default, all transactions which pass through the gateway will be automatically considered
        // as received.
        None
    }

    async fn subscribe_new_transactions(
        &self,
    ) -> Option<tokio::sync::broadcast::Receiver<starknet_types_core::felt::Felt>> {
        // We cannot subscribe to new transactions from the gateway for the same reasons as above
        None
    }
}
