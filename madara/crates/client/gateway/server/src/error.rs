use super::helpers::internal_error_response;
use crate::helpers::{create_json_response, not_found_response};
use hyper::Response;
use mc_db::view::block_id::BlockResolutionError;
use mc_rpc::StarknetRpcApiError;
use mc_submit_tx::{RejectedTransactionError, RejectedTransactionErrorKind, SubmitTransactionError};
use mp_gateway::error::{StarknetError, StarknetErrorCode};
use std::borrow::Cow;

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("Unsupported operation")]
    Unsupported,
    #[error(transparent)]
    StarknetError(#[from] StarknetError),
    #[error("Internal server error")]
    InternalServerError,
}

impl From<BlockResolutionError> for GatewayError {
    fn from(value: BlockResolutionError) -> Self {
        match value {
            BlockResolutionError::NoBlocks => todo!(),
            BlockResolutionError::BlockHashNotFound | BlockResolutionError::BlockNumberNotFound => {
                StarknetError::block_not_found().into()
            }
            BlockResolutionError::Internal(error) => error.into(),
        }
    }
}

impl From<anyhow::Error> for GatewayError {
    fn from(e: anyhow::Error) -> Self {
        tracing::error!(target: "gateway_errors", "Internal gateway server error: {:#}", e);
        Self::InternalServerError
    }
}

impl From<GatewayError> for Response<String> {
    fn from(e: GatewayError) -> Response<String> {
        match e {
            GatewayError::StarknetError(e) => create_json_response(hyper::StatusCode::BAD_REQUEST, &e),
            GatewayError::InternalServerError => internal_error_response(),
            GatewayError::Unsupported => not_found_response(),
        }
    }
}

fn map_rejected_tx_error(value: RejectedTransactionError) -> StarknetError {
    use RejectedTransactionErrorKind as E;
    use StarknetErrorCode::*;

    let code = match value.kind {
        E::EntryPointNotFound => EntryPointNotFound,
        E::OutOfRangeContractAddress => OutOfRangeContractAddress,
        E::TransactionFailed => TransactionFailed,
        E::UninitializedContract => UninitializedContract,
        E::OutOfRangeTransactionHash => OutOfRangeTransactionHash,
        E::UnsupportedSelectorForFee => UnsupportedSelectorForFee,
        E::InvalidContractDefinition => InvalidContractDefinition,
        E::NotPermittedContract => NotPermittedContract,
        E::UndeclaredClass => UndeclaredClass,
        E::TransactionLimitExceeded => TransactionLimitExceeded,
        E::InvalidTransactionNonce => InvalidTransactionNonce,
        E::OutOfRangeFee => OutOfRangeFee,
        E::InvalidTransactionVersion => InvalidTransactionVersion,
        E::InvalidProgram => InvalidProgram,
        E::DeprecatedTransaction => DeprecatedTransaction,
        E::InvalidCompiledClassHash => InvalidCompiledClassHash,
        E::CompilationFailed => CompilationFailed,
        E::UnauthorizedEntryPointForInvoke => UnauthorizedEntryPointForInvoke,
        E::InvalidContractClass => InvalidContractClass,
        E::ClassAlreadyDeclared => ClassAlreadyDeclared,
        E::InvalidSignature => InvalidSignature,
        E::InsufficientAccountBalance => InsufficientAccountBalance,
        E::InsufficientMaxFee => InsufficientMaxFee,
        E::ValidateFailure => ValidateFailure,
        E::ContractBytecodeSizeTooLarge => ContractBytecodeSizeTooLarge,
        E::ContractClassObjectSizeTooLarge => ContractClassObjectSizeTooLarge,
        E::DuplicatedTransaction => DuplicatedTransaction,
        E::InvalidContractClassVersion => InvalidContractClassVersion,
        E::RateLimited => RateLimited,
    };
    StarknetError { code, message: value.message.unwrap_or_default().into() }
}

impl From<SubmitTransactionError> for GatewayError {
    fn from(value: SubmitTransactionError) -> Self {
        match value {
            SubmitTransactionError::Unsupported => Self::Unsupported,
            SubmitTransactionError::Rejected(rejected_transaction_error) => {
                Self::StarknetError(map_rejected_tx_error(rejected_transaction_error))
            }
            SubmitTransactionError::Internal(error) => error.into(),
        }
    }
}

impl From<StarknetRpcApiError> for GatewayError {
    fn from(e: StarknetRpcApiError) -> Self {
        fn err_message(error: Cow<'static, str>, or: &'static str) -> String {
            if error.is_empty() {
                or.into()
            } else {
                error.into()
            }
        }
        match e {
            StarknetRpcApiError::InternalServerError => GatewayError::InternalServerError,
            StarknetRpcApiError::BlockNotFound => GatewayError::StarknetError(StarknetError::block_not_found()),
            StarknetRpcApiError::InvalidContractClass { error } => GatewayError::StarknetError(StarknetError::new(
                StarknetErrorCode::InvalidContractClass,
                err_message(error, "Invalid contract class"),
            )),
            StarknetRpcApiError::ClassAlreadyDeclared { error } => GatewayError::StarknetError(StarknetError::new(
                StarknetErrorCode::ClassAlreadyDeclared,
                err_message(error, "Class already declared"),
            )),
            StarknetRpcApiError::InsufficientMaxFee { error } => GatewayError::StarknetError(StarknetError::new(
                StarknetErrorCode::InsufficientMaxFee,
                err_message(error, "Insufficient max fee"),
            )),
            StarknetRpcApiError::InsufficientAccountBalance { error } => {
                GatewayError::StarknetError(StarknetError::new(
                    StarknetErrorCode::InsufficientAccountBalance,
                    err_message(error, "Insufficient account balance"),
                ))
            }
            StarknetRpcApiError::ValidationFailure { error } => {
                GatewayError::StarknetError(StarknetError::new(StarknetErrorCode::ValidateFailure, error.into()))
            }
            StarknetRpcApiError::CompilationFailed { error } => GatewayError::StarknetError(StarknetError::new(
                StarknetErrorCode::CompilationFailed,
                err_message(error, "Compilation failed"),
            )),
            StarknetRpcApiError::ContractClassSizeTooLarge { error } => {
                GatewayError::StarknetError(StarknetError::new(
                    StarknetErrorCode::ContractBytecodeSizeTooLarge,
                    err_message(error, "Contract class size is too large"),
                ))
            }
            StarknetRpcApiError::NonAccount { error } => GatewayError::StarknetError(StarknetError::new(
                StarknetErrorCode::NotPermittedContract,
                err_message(error, "Sender address is not an account contract"),
            )),
            StarknetRpcApiError::DuplicateTxn { error } => GatewayError::StarknetError(StarknetError::new(
                StarknetErrorCode::DuplicatedTransaction,
                err_message(error, "A transaction with the same hash already exists in the mempool"),
            )),
            StarknetRpcApiError::CompiledClassHashMismatch { error } => {
                GatewayError::StarknetError(StarknetError::new(
                    StarknetErrorCode::InvalidCompiledClassHash,
                    err_message(error, "The compiled class hash did not match the one supplied in the transaction"),
                ))
            }
            StarknetRpcApiError::UnsupportedTxnVersion { error } => GatewayError::StarknetError(StarknetError::new(
                StarknetErrorCode::InvalidTransactionVersion,
                err_message(error, "The transaction version is not supported"),
            )),
            StarknetRpcApiError::UnsupportedContractClassVersion { error } => {
                GatewayError::StarknetError(StarknetError::new(
                    StarknetErrorCode::InvalidContractClassVersion,
                    err_message(error, "The contract class version is not supported"),
                ))
            }
            StarknetRpcApiError::ErrUnexpectedError { error } => GatewayError::StarknetError(StarknetError::new(
                StarknetErrorCode::TransactionFailed,
                format!("An unexpected error occurred: {}", error),
            )),
            e => anyhow::Error::from(e).into(),
        }
    }
}
