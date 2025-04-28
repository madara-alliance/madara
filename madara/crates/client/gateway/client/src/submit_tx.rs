use std::borrow::Cow;

use crate::GatewayProvider;
use async_trait::async_trait;
use mc_submit_tx::{RejectedTransactionError, RejectedTransactionErrorKind, SubmitTransaction, SubmitTransactionError};
use mp_gateway::{error::SequencerError, user_transaction::UserTransactionConversionError};
use mp_rpc::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn,
    ClassAndTxnHash, ContractAndTxnHash,
};

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

#[async_trait]
impl SubmitTransaction for GatewayProvider {
    async fn submit_declare_transaction(
        &self,
        tx: BroadcastedDeclareTxn,
    ) -> Result<ClassAndTxnHash, SubmitTransactionError> {
        self.add_declare_transaction(tx.try_into().map_err(map_conv_error)?)
            .await
            .map_err(map_gateway_error)
            .map(|res| ClassAndTxnHash { transaction_hash: res.transaction_hash, class_hash: res.class_hash })
    }

    async fn submit_deploy_account_transaction(
        &self,
        tx: BroadcastedDeployAccountTxn,
    ) -> Result<ContractAndTxnHash, SubmitTransactionError> {
        self.add_deploy_account_transaction(tx.try_into().map_err(map_conv_error)?)
            .await
            .map_err(map_gateway_error)
            .map(|res| ContractAndTxnHash { transaction_hash: res.transaction_hash, contract_address: res.address })
    }

    async fn submit_invoke_transaction(
        &self,
        tx: BroadcastedInvokeTxn,
    ) -> Result<AddInvokeTransactionResult, SubmitTransactionError> {
        self.add_invoke_transaction(tx.try_into().map_err(map_conv_error)?)
            .await
            .map_err(map_gateway_error)
            .map(|res| AddInvokeTransactionResult { transaction_hash: res.transaction_hash })
    }
}

#[async_trait]
impl mc_submit_tx::SubmitValidatedTransaction for GatewayProvider {
    async fn submit_validated_transaction(
        &self,
        tx: mp_transactions::validated::ValidatedMempoolTx,
    ) -> Result<(), SubmitTransactionError> {
        self.add_validated_transaction(tx).await.map_err(map_gateway_error)
    }
}
