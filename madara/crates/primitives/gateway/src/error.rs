use std::fmt::Display;

use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;
use starknet_types_core::felt::FromStrError;

#[derive(Debug, thiserror::Error)]
pub enum SequencerError {
    #[error("Starknet error: {0:#}")]
    StarknetError(#[from] StarknetError),
    #[error("Hyper error: {0:#}")]
    HyperError(#[from] hyper::Error),
    #[error("No URL available to use this request")]
    NoUrl,
    /// The URL stored here must already be redacted with [`redact_url_for_logging`]: operators
    /// commonly embed API keys in custom gateway URLs (`--gateway-url`), and this error (like the
    /// whole [`SequencerError`] chain) ends up in logs and error contexts.
    #[error("Invalid URL: {0}")]
    InvalidUrl(String),
    #[error("HTTP error: {0:#}")]
    HttpError(#[from] hyper::http::Error),
    #[error("Error calling HTTP client: {0:#}")]
    HttpCallError(Box<dyn std::error::Error + Send + Sync>),
    #[error("Error deserializing response: {serde_error:#}")]
    DeserializeBody { serde_error: serde_json::Error },
    #[error("Serialization or deserialization error: {0:#}")]
    SerializeRequest(#[from] serde_json::Error),
    #[error("Error compressing class: {0:#}")]
    CompressError(#[from] starknet_core::types::contract::CompressProgramError),
    #[error("Failed to parse returned error with http status {http_status}: {serde_error:#}")]
    InvalidStarknetError { http_status: StatusCode, serde_error: serde_json::Error },
}

/// Renders a gateway URL so that it is safe to embed in logs and error messages.
///
/// Operators commonly embed API keys in custom gateway URLs (`--gateway-url`), either as
/// userinfo (`https://user:key@host/...`), as a query parameter (`?apikey=...`), or as a path
/// segment (`https://host/<api-key>/feeder_gateway`, the style used by most RPC providers).
/// Anything that could carry a credential is masked:
///
/// - userinfo (username and password) is replaced with `***`,
/// - every query parameter *value* is masked (parameter names are kept),
/// - path segments that look like tokens (16+ characters of `[A-Za-z0-9_-]` containing at least
///   one digit) are masked. Standard gateway path segments (`feeder_gateway`, `get_block`,
///   `get_compiled_class_by_class_hash`, ...) contain no digits and are preserved.
///
/// Note that the `--gateway-key` flag is sent as an HTTP header and never appears in URLs; this
/// helper protects keys embedded in the URL itself.
pub fn redact_url_for_logging(url: &url::Url) -> String {
    fn is_token_like(segment: &str) -> bool {
        segment.len() >= 16
            && segment.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            && segment.chars().any(|c| c.is_ascii_digit())
    }

    let mut url = url.clone();
    if url.password().is_some() {
        let _ = url.set_password(Some("***"));
    }
    if !url.username().is_empty() {
        let _ = url.set_username("***");
    }
    if url.query().is_some() {
        let masked = url.query_pairs().map(|(key, _)| format!("{key}=***")).collect::<Vec<_>>().join("&");
        url.set_query(Some(&masked));
    }
    if url.path_segments().is_some_and(|mut segments| segments.any(is_token_like)) {
        let masked = url
            .path_segments()
            .expect("checked by the condition above")
            .map(|segment| if is_token_like(segment) { "***" } else { segment })
            .collect::<Vec<_>>()
            .join("/");
        url.set_path(&masked);
    }
    url.to_string()
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
    pub(crate) const MISSING_CLASS_HASH: &str = "Missing classHash parameter";
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

    pub fn malformed_request(e: impl Display) -> Self {
        Self { code: StarknetErrorCode::MalformedRequest, message: format!("Failed to parse transaction: {:#}", e) }
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
    #[serde(rename = "StarknetErrorCode.REPLACEMENT_TRANSACTION_UNDERPRICED")]
    ReplacementTransactionUnderpriced,
    #[serde(rename = "StarknetErrorCode.FEE_BELOW_MINIMUM")]
    FeeBelowMinimum,
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
    #[serde(rename = "StarknetErrorCode.INVALID_PROOF")]
    InvalidProof,
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

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    #[test]
    fn redact_url_masks_query_param_values() {
        let url = Url::parse("https://gateway.example.com/feeder_gateway/get_block?apikey=SECRETKEY123&x=1").unwrap();
        let redacted = redact_url_for_logging(&url);
        assert!(!redacted.contains("SECRETKEY123"), "{redacted}");
        assert_eq!(redacted, "https://gateway.example.com/feeder_gateway/get_block?apikey=***&x=***");
    }

    #[test]
    fn redact_url_masks_userinfo() {
        let url = Url::parse("https://user:hunter2@gateway.example.com/feeder_gateway").unwrap();
        let redacted = redact_url_for_logging(&url);
        assert!(!redacted.contains("hunter2"), "{redacted}");
        assert!(!redacted.contains("user:"), "{redacted}");
        assert_eq!(redacted, "https://***:***@gateway.example.com/feeder_gateway");
    }

    #[test]
    fn redact_url_masks_token_like_path_segments() {
        // RPC-provider style URL with the project API key as a path segment.
        let url =
            Url::parse("https://starknet-mainnet.example.io/8237aafe06d8f6213d839e25a8637b3c/feeder_gateway").unwrap();
        let redacted = redact_url_for_logging(&url);
        assert!(!redacted.contains("8237aafe06d8f6213d839e25a8637b3c"), "{redacted}");
        assert_eq!(redacted, "https://starknet-mainnet.example.io/***/feeder_gateway");
    }

    #[test]
    fn redact_url_keeps_normal_gateway_urls_readable() {
        let url =
            Url::parse("https://feeder.alpha-mainnet.starknet.io/feeder_gateway/get_compiled_class_by_class_hash")
                .unwrap();
        assert_eq!(redact_url_for_logging(&url), url.as_str());
    }

    #[test]
    fn invalid_url_error_string_does_not_leak_api_key() {
        let url =
            Url::parse("https://host.example.com/0123456789abcdef0123456789abcdef/feeder_gateway?api_key=TOPSECRET9")
                .unwrap();
        let error = SequencerError::InvalidUrl(redact_url_for_logging(&url));
        let message = error.to_string();
        assert!(!message.contains("0123456789abcdef"), "{message}");
        assert!(!message.contains("TOPSECRET9"), "{message}");
        assert!(message.contains("host.example.com"), "{message}");
    }
}
