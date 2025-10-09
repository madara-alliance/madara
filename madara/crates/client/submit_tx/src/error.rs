use std::{borrow::Cow, fmt};

#[derive(Debug, thiserror::Error)]
pub enum SubmitTransactionError {
    /// Currently only returned when trying to add a validated transaction to a gateway that doesn't support or allow it.
    #[error("Unsupported operation")]
    Unsupported,
    /// Validation failed, or any other expected error.
    #[error("Transaction rejected: {0}")]
    Rejected(#[from] RejectedTransactionError),
    /// Any internal error. Note that when redirecting transactions from one node to another,
    /// any transport/connectivity/unexpected gateway errors will appear as internal here.
    #[error("Internal error: {0:#}")]
    Internal(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub struct RejectedTransactionError {
    pub kind: RejectedTransactionErrorKind,
    pub message: Option<Cow<'static, str>>,
}

impl RejectedTransactionError {
    /// Use [`From`] to get a [`RejectedTransactionError`] without a message.
    pub fn new(kind: RejectedTransactionErrorKind, message: impl Into<Cow<'static, str>>) -> Self {
        Self { kind, message: Some(message.into()) }
    }
}

impl From<RejectedTransactionErrorKind> for RejectedTransactionError {
    fn from(kind: RejectedTransactionErrorKind) -> Self {
        Self { kind, message: None }
    }
}

impl fmt::Display for RejectedTransactionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(message) = self.message.as_ref().filter(|m| !m.is_empty()) {
            write!(f, "{}: {}", self.kind, message)
        } else {
            write!(f, "{}", self.kind)
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RejectedTransactionErrorKind {
    #[error("EntryPointNotFound")]
    EntryPointNotFound,
    #[error("OutOfRangeContractAddress")]
    OutOfRangeContractAddress,
    #[error("TransactionFailed")]
    TransactionFailed,
    #[error("UninitializedContract")]
    UninitializedContract,
    #[error("OutOfRangeTransactionHash")]
    OutOfRangeTransactionHash,
    #[error("UnsupportedSelectorForFee")]
    UnsupportedSelectorForFee,
    #[error("InvalidContractDefinition")]
    InvalidContractDefinition,
    #[error("NotPermittedContract")]
    NotPermittedContract,
    #[error("UndeclaredClass")]
    UndeclaredClass,
    #[error("TransactionLimitExceeded")]
    TransactionLimitExceeded,
    #[error("InvalidTransactionNonce")]
    InvalidTransactionNonce,
    #[error("Replacement transaction is underpriced")]
    ReplacementTransactionUnderpriced,
    #[error("Transaction fee below minimum")]
    FeeBelowMinimum,
    #[error("OutOfRangeFee")]
    OutOfRangeFee,
    #[error("InvalidTransactionVersion")]
    InvalidTransactionVersion,
    #[error("InvalidProgram")]
    InvalidProgram,
    #[error("DeprecatedTransaction")]
    DeprecatedTransaction,
    #[error("InvalidCompiledClassHash")]
    InvalidCompiledClassHash,
    #[error("CompilationFailed")]
    CompilationFailed,
    #[error("UnauthorizedEntryPointForInvoke")]
    UnauthorizedEntryPointForInvoke,
    #[error("InvalidContractClass")]
    InvalidContractClass,
    #[error("ClassAlreadyDeclared")]
    ClassAlreadyDeclared,
    #[error("InvalidSignature")]
    InvalidSignature,
    #[error("InsufficientAccountBalance")]
    InsufficientAccountBalance,
    #[error("InsufficientMaxFee")]
    InsufficientMaxFee,
    #[error("ValidateFailure")]
    ValidateFailure,
    #[error("ContractBytecodeSizeTooLarge")]
    ContractBytecodeSizeTooLarge,
    #[error("ContractClassObjectSizeTooLarge")]
    ContractClassObjectSizeTooLarge,
    #[error("DuplicatedTransaction")]
    DuplicatedTransaction,
    #[error("InvalidContractClassVersion")]
    InvalidContractClassVersion,
    #[error("RateLimited")]
    RateLimited,
}
