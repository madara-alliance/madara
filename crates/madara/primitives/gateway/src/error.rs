use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;
use starknet_types_core::felt::FromStrError;

#[derive(Debug, thiserror::Error)]
pub enum SequencerError {
    #[error("Starknet error: {0:#}")]
    StarknetError(#[from] StarknetError),
    #[error("Hyper error: {0:#}")]
    ReqwestError(#[from] hyper::Error),
    #[error("Invalid URL: {0}")]
    InvalidUrl(url::Url),
    #[error("HTTP error: {0:#}")]
    HttpError(#[from] hyper::http::Error),
    #[error("Error calling HTTP client: {0:#}")]
    HttpCallError(Box<dyn std::error::Error + Send + Sync>),
    #[error("Error deserializing response: {serde_error:#}")]
    DeserializeBody { serde_error: serde_json::Error },
    #[error("Error serializing request: {0:#}")]
    SerializeRequest(#[from] serde_json::Error),
    #[error("Error compressing class: {0:#}")]
    CompressError(#[from] starknet_core::types::contract::CompressProgramError),
    #[error("Failed to parse returned error with http status {http_status}: {serde_error:#}")]
    InvalidStarknetError { http_status: StatusCode, serde_error: serde_json::Error },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StarknetError {
    pub code: StarknetErrorCode,
    pub message: String,
}

mod err {
    pub(crate) const RATE_LIMITED: &str = "Too many requests";
    pub(crate) const BLOCK_NOT_FOUND: &str = "Block not found";
    pub(crate) const NO_SIGNATURE_FOR_PENDING_BLOCK: &str =
        "BlockSignature is not supported for pending blocks; try querying with a concrete block identifier";
    pub(crate) const NO_BLOCK_HEADER_FOR_PENDING_BLOCK: &str = "Block header is not supported for the pending block";
    pub(crate) const MISSING_CLASS_HASH: &str = "Missing class_hash parameter";
}

impl StarknetError {
    pub fn new(code: StarknetErrorCode, message: String) -> Self {
        Self { code, message }
    }

    pub fn rate_limited() -> Self {
        Self { code: StarknetErrorCode::RateLimited, message: err::RATE_LIMITED.to_string() }
    }

    pub fn block_not_found() -> Self {
        Self { code: StarknetErrorCode::BlockNotFound, message: err::BLOCK_NOT_FOUND.to_string() }
    }

    pub fn no_signature_for_pending_block() -> Self {
        Self {
            code: StarknetErrorCode::NoSignatureForPendingBlock,
            message: err::NO_SIGNATURE_FOR_PENDING_BLOCK.to_string(),
        }
    }

    pub fn no_block_header_for_pending_block() -> Self {
        Self { code: StarknetErrorCode::NoBlockHeader, message: err::NO_BLOCK_HEADER_FOR_PENDING_BLOCK.to_string() }
    }

    pub fn missing_class_hash() -> Self {
        Self { code: StarknetErrorCode::MalformedRequest, message: err::MISSING_CLASS_HASH.to_string() }
    }

    pub fn invalid_class_hash(e: FromStrError) -> Self {
        Self { code: StarknetErrorCode::MalformedRequest, message: format!("Invalid class_hash: {}", e) }
    }

    pub fn class_not_found(class_hash: Felt) -> Self {
        Self {
            code: StarknetErrorCode::UndeclaredClass,
            message: format!("Class with hash {:#x} not found", class_hash),
        }
    }

    pub fn sierra_class_not_found(class_hash: Felt) -> Self {
        Self {
            code: StarknetErrorCode::UndeclaredClass,
            message: format!("Class with hash {:#x} is not a sierra class", class_hash),
        }
    }

    pub fn malformed_request(e: serde_json::Error) -> Self {
        Self { code: StarknetErrorCode::MalformedRequest, message: format!("Failed to parse transaction: {}", e) }
    }
}

impl std::error::Error for StarknetError {}

impl std::fmt::Display for StarknetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

#[derive(Copy, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub enum StarknetErrorCode {
    #[serde(rename = "StarknetErrorCode.BLOCK_NOT_FOUND")]
    BlockNotFound,
    #[serde(rename = "StarknetErrorCode.NO_BLOCK_HEADER")]
    NoBlockHeader,
    #[serde(rename = "StarknetErrorCode.ENTRY_POINT_NOT_FOUND_IN_CONTRACT")]
    EntryPointNotFound,
    #[serde(rename = "StarknetErrorCode.OUT_OF_RANGE_CONTRACT_ADDRESS")]
    OutOfRangeContractAddress,
    #[serde(rename = "StarkErrorCode.SCHEMA_VALIDATION_ERROR")]
    SchemaValidationError,
    #[serde(rename = "StarknetErrorCode.TRANSACTION_FAILED")]
    TransactionFailed,
    #[serde(rename = "StarknetErrorCode.UNINITIALIZED_CONTRACT")]
    UninitializedContract,
    #[serde(rename = "StarknetErrorCode.OUT_OF_RANGE_BLOCK_HASH")]
    OutOfRangeBlockHash,
    #[serde(rename = "StarknetErrorCode.OUT_OF_RANGE_TRANSACTION_HASH")]
    OutOfRangeTransactionHash,
    #[serde(rename = "StarkErrorCode.MALFORMED_REQUEST")]
    MalformedRequest,
    #[serde(rename = "StarknetErrorCode.UNSUPPORTED_SELECTOR_FOR_FEE")]
    UnsupportedSelectorForFee,
    #[serde(rename = "StarknetErrorCode.INVALID_CONTRACT_DEFINITION")]
    InvalidContractDefinition,
    #[serde(rename = "StarknetErrorCode.NON_PERMITTED_CONTRACT")]
    NotPermittedContract,
    #[serde(rename = "StarknetErrorCode.UNDECLARED_CLASS")]
    UndeclaredClass,
    #[serde(rename = "StarknetErrorCode.TRANSACTION_LIMIT_EXCEEDED")]
    TransactionLimitExceeded,
    #[serde(rename = "StarknetErrorCode.INVALID_TRANSACTION_NONCE")]
    InvalidTransactionNonce,
    #[serde(rename = "StarknetErrorCode.OUT_OF_RANGE_FEE")]
    OutOfRangeFee,
    #[serde(rename = "StarknetErrorCode.INVALID_TRANSACTION_VERSION")]
    InvalidTransactionVersion,
    #[serde(rename = "StarknetErrorCode.INVALID_PROGRAM")]
    InvalidProgram,
    #[serde(rename = "StarknetErrorCode.DEPRECATED_TRANSACTION")]
    DeprecatedTransaction,
    #[serde(rename = "StarknetErrorCode.INVALID_COMPILED_CLASS_HASH")]
    InvalidCompiledClassHash,
    #[serde(rename = "StarknetErrorCode.COMPILATION_FAILED")]
    CompilationFailed,
    #[serde(rename = "StarknetErrorCode.UNAUTHORIZED_ENTRY_POINT_FOR_INVOKE")]
    UnauthorizedEntryPointForInvoke,
    #[serde(rename = "StarknetErrorCode.INVALID_CONTRACT_CLASS")]
    InvalidContractClass,
    #[serde(rename = "StarknetErrorCode.CLASS_ALREADY_DECLARED")]
    ClassAlreadyDeclared,
    #[serde(rename = "StarknetErrorCode.INVALID_SIGNATURE")]
    InvalidSignature,
    #[serde(rename = "StarknetErrorCode.NO_SIGNATURE_FOR_PENDING_BLOCK")]
    NoSignatureForPendingBlock,
    #[serde(rename = "StarknetErrorCode.INSUFFICIENT_ACCOUNT_BALANCE")]
    InsufficientAccountBalance,
    #[serde(rename = "StarknetErrorCode.INSUFFICIENT_MAX_FEE")]
    InsufficientMaxFee,
    #[serde(rename = "StarknetErrorCode.VALIDATE_FAILURE")]
    ValidateFailure,
    #[serde(rename = "StarknetErrorCode.CONTRACT_BYTECODE_SIZE_TOO_LARGE")]
    ContractBytecodeSizeTooLarge,
    #[serde(rename = "StarknetErrorCode.CONTRACT_CLASS_OBJECT_SIZE_TOO_LARGE")]
    ContractClassObjectSizeTooLarge,
    #[serde(rename = "StarknetErrorCode.DUPLICATED_TRANSACTION")]
    DuplicatedTransaction,
    #[serde(rename = "StarknetErrorCode.INVALID_CONTRACT_CLASS_VERSION")]
    InvalidContractClassVersion,
    #[serde(rename = "StarknetErrorCode.RATE_LIMITED")]
    RateLimited,
}
