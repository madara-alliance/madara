use crate::utils::display_internal_server_error;
use mc_db::MadaraStorageError;
use mp_gateway::error::{StarknetError, StarknetErrorCode};
use mp_gateway::user_transaction::UserTransactionConversionError;
use serde::Serialize;
use serde_json::json;
use starknet_api::StarknetApiError;
use starknet_types_core::felt::Felt;
use std::borrow::Cow;
use std::fmt::Display;

pub type StarknetRpcResult<T> = Result<T, StarknetRpcApiError>;

pub enum StarknetTransactionExecutionError {
    ContractNotFound,
    ClassAlreadyDeclared,
    ClassHashNotFound,
    InvalidContractClass,
    ContractError,
}

#[derive(Clone, Copy, Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StorageProofLimit {
    MaxUsedTries,
    MaxKeys,
}

#[derive(Clone, Copy, Serialize, Debug, PartialEq, Eq)]
#[serde(tag = "trie", content = "contract_address", rename_all = "snake_case")]
pub enum StorageProofTrie {
    Classes,
    Contracts,
    /// Associated Felt is the contract address.
    ContractStorage(Felt),
}

// Comes from the RPC Spec:
// https://github.com/starkware-libs/starknet-specs/blob/0e859ff905795f789f1dfd6f7340cdaf5015acc8/api/starknet_write_api.json#L227
#[cfg_attr(test, derive(PartialEq, Eq))]
#[derive(thiserror::Error, Debug)]
pub enum StarknetRpcApiError {
    #[error("Failed to write transaction")]
    FailedToReceiveTxn { err: Option<Cow<'static, str>> },
    #[error("Contract not found")]
    ContractNotFound,
    #[error("Block not found")]
    BlockNotFound,
    #[error("Invalid transaction hash")]
    InvalidTxnHash,
    #[error("Invalid tblock hash")]
    InvalidBlockHash,
    #[error("Invalid transaction index in a block")]
    InvalidTxnIndex,
    #[error("Class hash not found")]
    ClassHashNotFound,
    #[error("Transaction hash not found")]
    TxnHashNotFound,
    #[error("Requested page size is too big")]
    PageSizeTooBig,
    #[error("There are no blocks")]
    NoBlocks,
    #[error("The supplied continuation token is invalid or unknown")]
    InvalidContinuationToken,
    #[error("Too many keys provided in a filter")]
    TooManyKeysInFilter,
    #[error("Failed to fetch pending transactions")]
    FailedToFetchPendingTransactions,
    #[error("Contract error")]
    ContractError,
    #[error("Transaction execution error")]
    TxnExecutionError { tx_index: usize, error: String },
    #[error("Invalid contract class")]
    InvalidContractClass,
    #[error("Class already declared")]
    ClassAlreadyDeclared,
    #[error("Invalid transaction nonce")]
    InvalidTxnNonce,
    #[error("Max fee is smaller than the minimal transaction cost (validation plus fee transfer)")]
    InsufficientMaxFee,
    #[error("Account balance is smaller than the transaction's max_fee")]
    InsufficientAccountBalance,
    #[error("Account validation failed")]
    ValidationFailure { error: Cow<'static, str> },
    #[error("Compilation failed")]
    CompilationFailed,
    #[error("Contract class size is too large")]
    ContractClassSizeTooLarge,
    #[error("Sender address is not an account contract")]
    NonAccount,
    #[error("A transaction with the same hash already exists in the mempool")]
    DuplicateTxn,
    #[error("The compiled class hash did not match the one supplied in the transaction")]
    CompiledClassHashMismatch,
    #[error("The transaction version is not supported")]
    UnsupportedTxnVersion,
    #[error("The contract class version is not supported")]
    UnsupportedContractClassVersion,
    #[error("An unexpected error occurred")]
    ErrUnexpectedError { data: String },
    #[error("Internal server error")]
    InternalServerError,
    #[error("Unimplemented method")]
    UnimplementedMethod,
    #[error("Proof limit exceeded")]
    ProofLimitExceeded { kind: StorageProofLimit, limit: usize, got: usize },
    #[error("Cannot create a storage proof for a block that old")]
    CannotMakeProofOnOldBlock,
}

impl From<&StarknetRpcApiError> for i32 {
    fn from(err: &StarknetRpcApiError) -> Self {
        match err {
            StarknetRpcApiError::FailedToReceiveTxn { .. } => 1,
            StarknetRpcApiError::ContractNotFound => 20,
            StarknetRpcApiError::BlockNotFound => 24,
            StarknetRpcApiError::InvalidTxnHash => 25,
            StarknetRpcApiError::InvalidBlockHash => 26,
            StarknetRpcApiError::InvalidTxnIndex => 27,
            StarknetRpcApiError::ClassHashNotFound => 28,
            StarknetRpcApiError::TxnHashNotFound => 29,
            StarknetRpcApiError::PageSizeTooBig => 31,
            StarknetRpcApiError::NoBlocks => 32,
            StarknetRpcApiError::InvalidContinuationToken => 33,
            StarknetRpcApiError::TooManyKeysInFilter => 34,
            StarknetRpcApiError::FailedToFetchPendingTransactions => 38,
            StarknetRpcApiError::ContractError => 40,
            StarknetRpcApiError::TxnExecutionError { .. } => 41,
            StarknetRpcApiError::InvalidContractClass => 50,
            StarknetRpcApiError::ClassAlreadyDeclared => 51,
            StarknetRpcApiError::InvalidTxnNonce => 52,
            StarknetRpcApiError::InsufficientMaxFee => 53,
            StarknetRpcApiError::InsufficientAccountBalance => 54,
            StarknetRpcApiError::ValidationFailure { .. } => 55,
            StarknetRpcApiError::CompilationFailed => 56,
            StarknetRpcApiError::ContractClassSizeTooLarge => 57,
            StarknetRpcApiError::NonAccount => 58,
            StarknetRpcApiError::DuplicateTxn => 59,
            StarknetRpcApiError::CompiledClassHashMismatch => 60,
            StarknetRpcApiError::UnsupportedTxnVersion => 61,
            StarknetRpcApiError::UnsupportedContractClassVersion => 62,
            StarknetRpcApiError::ErrUnexpectedError { .. } => 63,
            StarknetRpcApiError::InternalServerError => 500,
            StarknetRpcApiError::UnimplementedMethod => 501,
            StarknetRpcApiError::ProofLimitExceeded { .. } => 10000,
            StarknetRpcApiError::CannotMakeProofOnOldBlock => 10001,
        }
    }
}

impl StarknetRpcApiError {
    pub fn data(&self) -> Option<serde_json::Value> {
        match self {
            StarknetRpcApiError::ErrUnexpectedError { data } => Some(json!(data)),
            StarknetRpcApiError::ValidationFailure { error } => Some(json!(error)),
            StarknetRpcApiError::FailedToReceiveTxn { err } => err.as_ref().map(|err| json!(err)),
            StarknetRpcApiError::TxnExecutionError { tx_index, error } => Some(json!({
                "transaction_index": tx_index,
                "execution_error": error,
            })),
            StarknetRpcApiError::ProofLimitExceeded { kind, limit, got } => {
                Some(json!({ "kind": kind, "limit": limit, "got": got }))
            }
            _ => None,
        }
    }
}

impl From<mc_exec::Error> for StarknetRpcApiError {
    fn from(err: mc_exec::Error) -> Self {
        Self::TxnExecutionError { tx_index: 0, error: format!("{:#}", err) }
    }
}

impl From<StarknetTransactionExecutionError> for StarknetRpcApiError {
    fn from(err: StarknetTransactionExecutionError) -> Self {
        match err {
            StarknetTransactionExecutionError::ContractNotFound => StarknetRpcApiError::ContractNotFound,
            StarknetTransactionExecutionError::ClassAlreadyDeclared => StarknetRpcApiError::ClassAlreadyDeclared,
            StarknetTransactionExecutionError::ClassHashNotFound => StarknetRpcApiError::ClassHashNotFound,
            StarknetTransactionExecutionError::InvalidContractClass => StarknetRpcApiError::InvalidContractClass,
            StarknetTransactionExecutionError::ContractError => StarknetRpcApiError::ContractError,
        }
    }
}

impl From<StarknetRpcApiError> for jsonrpsee::types::ErrorObjectOwned {
    fn from(err: StarknetRpcApiError) -> Self {
        jsonrpsee::types::ErrorObjectOwned::owned((&err).into(), err.to_string(), err.data())
    }
}

impl From<StarknetError> for StarknetRpcApiError {
    fn from(err: StarknetError) -> Self {
        match err.code {
            StarknetErrorCode::BlockNotFound => StarknetRpcApiError::BlockNotFound,
            StarknetErrorCode::TransactionFailed => {
                StarknetRpcApiError::FailedToReceiveTxn { err: Some(err.message.into()) }
            }
            StarknetErrorCode::ValidateFailure => StarknetRpcApiError::ValidationFailure { error: err.message.into() },
            StarknetErrorCode::UninitializedContract => StarknetRpcApiError::ContractNotFound,
            StarknetErrorCode::UndeclaredClass => StarknetRpcApiError::ClassHashNotFound,
            StarknetErrorCode::InvalidTransactionNonce => StarknetRpcApiError::InvalidTxnNonce,
            StarknetErrorCode::ClassAlreadyDeclared => StarknetRpcApiError::ClassAlreadyDeclared,
            StarknetErrorCode::CompilationFailed => StarknetRpcApiError::CompilationFailed,
            StarknetErrorCode::InvalidCompiledClassHash => StarknetRpcApiError::CompiledClassHashMismatch,
            StarknetErrorCode::DuplicatedTransaction => StarknetRpcApiError::DuplicateTxn,
            StarknetErrorCode::ContractBytecodeSizeTooLarge => StarknetRpcApiError::ContractClassSizeTooLarge,
            StarknetErrorCode::InvalidContractClassVersion => StarknetRpcApiError::UnsupportedContractClassVersion,
            StarknetErrorCode::InvalidContractClass => StarknetRpcApiError::InvalidContractClass,
            StarknetErrorCode::InvalidContractDefinition => StarknetRpcApiError::InvalidContractClass,
            StarknetErrorCode::OutOfRangeBlockHash => StarknetRpcApiError::InvalidBlockHash,
            StarknetErrorCode::OutOfRangeTransactionHash => StarknetRpcApiError::InvalidTxnHash,
            StarknetErrorCode::InvalidTransactionVersion => StarknetRpcApiError::UnsupportedTxnVersion,
            _ => StarknetRpcApiError::ErrUnexpectedError { data: err.message },
        }
    }
}

impl From<MadaraStorageError> for StarknetRpcApiError {
    fn from(_: MadaraStorageError) -> Self {
        StarknetRpcApiError::ErrUnexpectedError { data: "DB error".to_string() }
    }
}

impl From<StarknetApiError> for StarknetRpcApiError {
    fn from(err: StarknetApiError) -> Self {
        StarknetRpcApiError::ErrUnexpectedError { data: err.to_string() }
    }
}

impl From<UserTransactionConversionError> for StarknetRpcApiError {
    fn from(err: UserTransactionConversionError) -> Self {
        match err {
            UserTransactionConversionError::ContractClassDecodeError(_) => StarknetRpcApiError::InvalidContractClass,
            UserTransactionConversionError::UnsupportedQueryTransaction => StarknetRpcApiError::UnsupportedTxnVersion,
        }
    }
}

#[cfg_attr(test, derive(PartialEq, Eq))]
#[derive(Debug)]
pub enum StarknetWsApiError {
    TooManyBlocksBack,
    NoBlocks,
    BlockNotFound,
    Pending,
    Internal,
}

impl StarknetWsApiError {
    #[inline]
    fn code(&self) -> i32 {
        match self {
            Self::TooManyBlocksBack => 68,
            Self::NoBlocks => 32,
            Self::BlockNotFound => 24,
            Self::Pending => 69,
            Self::Internal => jsonrpsee::types::error::INTERNAL_ERROR_CODE,
        }
    }
    #[inline]
    fn message(&self) -> &str {
        match self {
            Self::TooManyBlocksBack => "Cannot go back more than 1024 blocks",
            Self::NoBlocks => "There are no blocks",
            Self::BlockNotFound => "Block not found",
            // See https://github.com/starkware-libs/starknet-specs/pull/237
            Self::Pending => "The pending block is not supported on this method call",
            Self::Internal => jsonrpsee::types::error::INTERNAL_ERROR_MSG,
        }
    }

    #[inline]
    pub fn internal_server_error<C: std::fmt::Display>(context: C) -> Self {
        display_internal_server_error(context);
        StarknetWsApiError::Internal
    }
}

impl Display for StarknetWsApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "\"code\": {}, \"message\": {}", self.code(), self.message())
    }
}

pub trait ErrorExtWs<T> {
    fn or_internal_server_error<C: std::fmt::Display>(self, context: C) -> Result<T, StarknetWsApiError>;

    fn or_else_internal_server_error<C: std::fmt::Display, F: FnOnce() -> C>(
        self,
        context_fn: F,
    ) -> Result<T, StarknetWsApiError>;
}

impl<T, E: std::fmt::Display> ErrorExtWs<T> for Result<T, E> {
    #[inline]
    fn or_internal_server_error<C: std::fmt::Display>(self, context: C) -> Result<T, StarknetWsApiError> {
        match self {
            Ok(res) => Ok(res),
            Err(err) => {
                display_internal_server_error(format!("{}: {:#}", context, err));
                Err(StarknetWsApiError::Internal)
            }
        }
    }

    #[inline]
    fn or_else_internal_server_error<C: std::fmt::Display, F: FnOnce() -> C>(
        self,
        context_fn: F,
    ) -> Result<T, StarknetWsApiError> {
        match self {
            Ok(res) => Ok(res),
            Err(err) => {
                display_internal_server_error(format!("{}: {:#}", context_fn(), err));
                Err(StarknetWsApiError::Internal)
            }
        }
    }
}

pub trait OptionExtWs<T> {
    #[allow(dead_code)]
    fn ok_or_internal_server_error<C: std::fmt::Display>(self, context: C) -> Result<T, StarknetWsApiError>;
    fn ok_or_else_internal_server_error<C: std::fmt::Display, F: FnOnce() -> C>(
        self,
        context_fn: F,
    ) -> Result<T, StarknetWsApiError>;
}

impl<T> OptionExtWs<T> for Option<T> {
    fn ok_or_internal_server_error<C: std::fmt::Display>(self, context: C) -> Result<T, StarknetWsApiError> {
        match self {
            Some(res) => Ok(res),
            None => {
                display_internal_server_error(context);
                Err(StarknetWsApiError::Internal)
            }
        }
    }

    fn ok_or_else_internal_server_error<C: std::fmt::Display, F: FnOnce() -> C>(
        self,
        context_fn: F,
    ) -> Result<T, StarknetWsApiError> {
        match self {
            Some(res) => Ok(res),
            None => {
                display_internal_server_error(context_fn());
                Err(StarknetWsApiError::Internal)
            }
        }
    }
}
