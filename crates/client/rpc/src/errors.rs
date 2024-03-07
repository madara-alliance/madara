use jsonrpsee::types::error::{CallError, ErrorObject};
use pallet_starknet_runtime_api::StarknetTransactionExecutionError;
use starknet_core::types::StarknetError;

// Comes from the RPC Spec:
// https://github.com/starkware-libs/starknet-specs/blob/0e859ff905795f789f1dfd6f7340cdaf5015acc8/api/starknet_write_api.json#L227
#[derive(thiserror::Error, Clone, Copy, Debug)]
pub enum StarknetRpcApiError {
    #[error("Failed to write transaction")]
    FailedToReceiveTxn = 1,
    #[error("Contract not found")]
    ContractNotFound = 20,
    #[error("Block not found")]
    BlockNotFound = 24,
    #[error("Invalid transaction hash")]
    InvalidTxnHash = 25,
    #[error("Invalid tblock hash")]
    InvalidBlockHash = 26,
    #[error("Invalid transaction index in a block")]
    InvalidTxnIndex = 27,
    #[error("Class hash not found")]
    ClassHashNotFound = 28,
    #[error("Transaction hash not found")]
    TxnHashNotFound = 29,
    #[error("Requested page size is too big")]
    PageSizeTooBig = 31,
    #[error("There are no blocks")]
    NoBlocks = 32,
    #[error("The supplied continuation token is invalid or unknown")]
    InvalidContinuationToken = 33,
    #[error("Too many keys provided in a filter")]
    TooManyKeysInFilter = 34,
    #[error("Failed to fetch pending transactions")]
    FailedToFetchPendingTransactions = 38,
    #[error("Contract error")]
    ContractError = 40,
    #[error("Transaction execution error")]
    TxnExecutionError = 41,
    #[error("Invalid contract class")]
    InvalidContractClass = 50,
    #[error("Class already declared")]
    ClassAlreadyDeclared = 51,
    #[error("Invalid transaction nonce")]
    InvalidTxnNonce = 52,
    #[error("Max fee is smaller than the minimal transaction cost (validation plus fee transfer)")]
    InsufficientMaxFee = 53,
    #[error("Account balance is smaller than the transaction's max_fee")]
    InsufficientAccountBalance = 54,
    #[error("Account validation failed")]
    ValidationFailure = 55,
    #[error("Compilation failed")]
    CompilationFailed = 56,
    #[error("Contract class size is too large")]
    ContractClassSizeTooLarge = 57,
    #[error("Sender address is not an account contract")]
    NonAccount = 58,
    #[error("A transaction with the same hash already exists in the mempool")]
    DuplicateTxn = 59,
    #[error("The compiled class hash did not match the one supplied in the transaction")]
    CompiledClassHashMismatch = 60,
    #[error("The transaction version is not supported")]
    UnsupportedTxnVersion = 61,
    #[error("The contract class version is not supported")]
    UnsupportedContractClassVersion = 62,
    #[error("An unexpected error occurred")]
    ErrUnexpectedError = 63,
    #[error("Internal server error")]
    InternalServerError = 500,
    #[error("Unimplemented method")]
    UnimplementedMethod = 501,
    #[error("Too many storage keys requested")]
    ProofLimitExceeded = 10000,
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

impl From<StarknetRpcApiError> for jsonrpsee::core::Error {
    fn from(err: StarknetRpcApiError) -> Self {
        jsonrpsee::core::Error::Call(CallError::Custom(ErrorObject::owned(err as i32, err.to_string(), None::<()>)))
    }
}

impl From<StarknetError> for StarknetRpcApiError {
    fn from(err: StarknetError) -> Self {
        match err {
            StarknetError::FailedToReceiveTransaction => StarknetRpcApiError::FailedToReceiveTxn,
            StarknetError::ContractNotFound => StarknetRpcApiError::ContractNotFound,
            StarknetError::BlockNotFound => StarknetRpcApiError::BlockNotFound,
            StarknetError::InvalidTransactionIndex => StarknetRpcApiError::InvalidTxnIndex,
            StarknetError::ClassHashNotFound => StarknetRpcApiError::ClassHashNotFound,
            StarknetError::TransactionHashNotFound => StarknetRpcApiError::TxnHashNotFound,
            StarknetError::PageSizeTooBig => StarknetRpcApiError::PageSizeTooBig,
            StarknetError::NoBlocks => StarknetRpcApiError::NoBlocks,
            StarknetError::InvalidContinuationToken => StarknetRpcApiError::InvalidContinuationToken,
            StarknetError::TooManyKeysInFilter => StarknetRpcApiError::TooManyKeysInFilter,
            StarknetError::ContractError(_) => StarknetRpcApiError::ContractError,
            StarknetError::ClassAlreadyDeclared => StarknetRpcApiError::ClassAlreadyDeclared,
            StarknetError::InvalidTransactionNonce => StarknetRpcApiError::InvalidTxnNonce,
            StarknetError::InsufficientMaxFee => StarknetRpcApiError::InsufficientMaxFee,
            StarknetError::InsufficientAccountBalance => StarknetRpcApiError::InsufficientAccountBalance,
            StarknetError::ValidationFailure => StarknetRpcApiError::ValidationFailure,
            StarknetError::CompilationFailed => StarknetRpcApiError::CompilationFailed,
            StarknetError::ContractClassSizeIsTooLarge => StarknetRpcApiError::ContractClassSizeTooLarge,
            StarknetError::NonAccount => StarknetRpcApiError::NonAccount,
            StarknetError::DuplicateTx => StarknetRpcApiError::DuplicateTxn,
            StarknetError::CompiledClassHashMismatch => StarknetRpcApiError::CompiledClassHashMismatch,
            StarknetError::UnsupportedTxVersion => StarknetRpcApiError::UnsupportedTxnVersion,
            StarknetError::UnsupportedContractClassVersion => StarknetRpcApiError::UnsupportedContractClassVersion,
            StarknetError::UnexpectedError(_) => StarknetRpcApiError::ErrUnexpectedError,
            StarknetError::NoTraceAvailable(_) => StarknetRpcApiError::InternalServerError,
            StarknetError::InvalidTransactionHash => StarknetRpcApiError::InvalidTxnHash,
        }
    }
}
