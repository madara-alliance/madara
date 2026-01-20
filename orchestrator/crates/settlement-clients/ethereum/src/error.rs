use alloy::transports::{RpcError, TransportErrorKind};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SendTransactionError {
    #[error("Replacement transaction underpriced: {0}")]
    #[allow(dead_code)]
    ReplacementTransactionUnderpriced(RpcError<TransportErrorKind>),

    #[error("Transaction retry limit reached: next multiplier ({next_mul:.2}x) exceeds maximum ({max_mul:.2}x)")]
    RetryLimitExceeded { next_mul: f64, max_mul: f64 },

    #[error("Error: {0}")]
    Other(#[from] RpcError<TransportErrorKind>),
}
