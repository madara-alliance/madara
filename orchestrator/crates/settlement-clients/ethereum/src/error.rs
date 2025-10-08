use alloy::transports::{RpcError, TransportErrorKind};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SendTransactionError {
    #[error("Replacement transaction underpriced: {0}")]
    #[allow(dead_code)]
    ReplacementTransactionUnderpriced(RpcError<TransportErrorKind>),
    #[error("Error: {0}")]
    Other(#[from] RpcError<TransportErrorKind>),
}
