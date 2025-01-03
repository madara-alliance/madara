use std::fmt::{self, Display};

use hyper::Response;
use mc_db::MadaraStorageError;
use mc_rpc::StarknetRpcApiError;
use mp_gateway::error::{StarknetError, StarknetErrorCode};

use crate::helpers::create_json_response;

use super::helpers::internal_error_response;

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error(transparent)]
    StarknetError(#[from] StarknetError),
    #[error("Internal server error: {0}")]
    InternalServerError(String),
}

impl From<MadaraStorageError> for GatewayError {
    fn from(e: MadaraStorageError) -> Self {
        tracing::error!(target: "gateway_errors", "Storage error: {}", e);
        Self::InternalServerError(e.to_string())
    }
}

impl From<GatewayError> for Response<String> {
    fn from(e: GatewayError) -> Response<String> {
        match e {
            GatewayError::StarknetError(e) => create_json_response(hyper::StatusCode::BAD_REQUEST, &e),
            GatewayError::InternalServerError(msg) => internal_error_response(&msg),
        }
    }
}

pub trait ResultExt<T, E> {
    fn or_internal_server_error<C: fmt::Display>(self, context: C) -> Result<T, GatewayError>;
}

impl<T, E: Display> ResultExt<T, E> for Result<T, E> {
    fn or_internal_server_error<C: fmt::Display>(self, context: C) -> Result<T, GatewayError> {
        match self {
            Ok(val) => Ok(val),
            Err(err) => {
                tracing::error!(target: "gateway_errors", "{context}: {err:#}");
                Err(GatewayError::InternalServerError(err.to_string()))
            }
        }
    }
}

pub trait OptionExt<T> {
    fn ok_or_internal_server_error<C: fmt::Display>(self, context: C) -> Result<T, GatewayError>;
}

impl<T> OptionExt<T> for Option<T> {
    fn ok_or_internal_server_error<C: fmt::Display>(self, context: C) -> Result<T, GatewayError> {
        match self {
            Some(val) => Ok(val),
            None => {
                tracing::error!(target: "gateway_errors", "{context}");
                Err(GatewayError::InternalServerError(context.to_string()))
            }
        }
    }
}

impl From<StarknetRpcApiError> for GatewayError {
    fn from(e: StarknetRpcApiError) -> Self {
        match e {
            StarknetRpcApiError::InternalServerError => {
                GatewayError::InternalServerError("Internal server error".to_string())
            }
            StarknetRpcApiError::BlockNotFound => GatewayError::StarknetError(StarknetError::block_not_found()),
            StarknetRpcApiError::InvalidContractClass => GatewayError::StarknetError(StarknetError::new(
                StarknetErrorCode::InvalidContractClass,
                "Invalid contract class".to_string(),
            )),
            StarknetRpcApiError::ClassAlreadyDeclared => GatewayError::StarknetError(StarknetError::new(
                StarknetErrorCode::ClassAlreadyDeclared,
                "Class already declared".to_string(),
            )),
            StarknetRpcApiError::InsufficientMaxFee => GatewayError::StarknetError(StarknetError::new(
                StarknetErrorCode::InsufficientMaxFee,
                "Insufficient max fee".to_string(),
            )),
            StarknetRpcApiError::InsufficientAccountBalance => GatewayError::StarknetError(StarknetError::new(
                StarknetErrorCode::InsufficientAccountBalance,
                "Insufficient account balance".to_string(),
            )),
            StarknetRpcApiError::ValidationFailure { error } => {
                GatewayError::StarknetError(StarknetError::new(StarknetErrorCode::ValidateFailure, error.into()))
            }
            StarknetRpcApiError::CompilationFailed => GatewayError::StarknetError(StarknetError::new(
                StarknetErrorCode::CompilationFailed,
                "Compilation failed".to_string(),
            )),
            StarknetRpcApiError::ContractClassSizeTooLarge => GatewayError::StarknetError(StarknetError::new(
                StarknetErrorCode::ContractBytecodeSizeTooLarge,
                "Contract class size is too large".to_string(),
            )),
            StarknetRpcApiError::NonAccount => GatewayError::StarknetError(StarknetError::new(
                StarknetErrorCode::NotPermittedContract,
                "Sender address is not an account contract".to_string(),
            )),
            StarknetRpcApiError::DuplicateTxn => GatewayError::StarknetError(StarknetError::new(
                StarknetErrorCode::DuplicatedTransaction,
                "A transaction with the same hash already exists in the mempool".to_string(),
            )),
            StarknetRpcApiError::CompiledClassHashMismatch => GatewayError::StarknetError(StarknetError::new(
                StarknetErrorCode::InvalidCompiledClassHash,
                "The compiled class hash did not match the one supplied in the transaction".to_string(),
            )),
            StarknetRpcApiError::UnsupportedTxnVersion => GatewayError::StarknetError(StarknetError::new(
                StarknetErrorCode::InvalidTransactionVersion,
                "The transaction version is not supported".to_string(),
            )),
            StarknetRpcApiError::UnsupportedContractClassVersion => GatewayError::StarknetError(StarknetError::new(
                StarknetErrorCode::InvalidContractClassVersion,
                "The contract class version is not supported".to_string(),
            )),
            StarknetRpcApiError::ErrUnexpectedError { data } => GatewayError::StarknetError(StarknetError::new(
                StarknetErrorCode::TransactionFailed,
                format!("An unexpected error occurred: {}", data),
            )),
            e => GatewayError::InternalServerError(format!("Unexpected error: {:#?}", e)),
        }
    }
}
