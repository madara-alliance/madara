use crate::error::job::fact::FactError;
use crate::error::other::OtherError;
use generate_pie::error::{BlockProcessingError, PieGenerationError};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SnosError {
    #[error("Block numbers to run must be specified (snos job #{internal_id:?})")]
    UnspecifiedBlockNumber { internal_id: u64 },
    #[error("No block numbers found (snos job #{internal_id:?})")]
    BlockNumberNotFound { internal_id: u64 },
    #[error("Invalid specified block number \"{block_number:?}\" (snos job #{internal_id:?})")]
    InvalidBlockNumber { internal_id: u64, block_number: String },

    #[error("Could not serialize the Cairo Pie (snos job #{internal_id:?}): {message}")]
    CairoPieUnserializable { internal_id: u64, message: String },
    #[error("Could not store the Cairo Pie (snos job #{internal_id:?}): {message}")]
    CairoPieUnstorable { internal_id: u64, message: String },

    #[error("Could not serialize the Snos Output (snos job #{internal_id:?}): {message}")]
    SnosOutputUnserializable { internal_id: u64, message: String },
    #[error("Could not serialize the Program Output (snos job #{internal_id:?}): {message}")]
    ProgramOutputUnserializable { internal_id: u64, message: String },
    #[error("Could not store the Snos output (snos job #{internal_id:?}): {message}")]
    SnosOutputUnstorable { internal_id: u64, message: String },
    #[error("Could not store the Program output (snos job #{internal_id:?}): {message}")]
    ProgramOutputUnstorable { internal_id: u64, message: String },

    #[error("Error while running SNOS (snos job #{internal_id:?}): {source}")]
    SnosExecutionError {
        internal_id: u64,
        #[source]
        source: PieGenerationError,
    },

    #[error("Error when calculating fact info: {0}")]
    FactCalculationError(#[from] FactError),

    #[error("Other error: {0}")]
    Other(#[from] OtherError),
    #[error("Could not serialize the On Chain Data (Snos job #{internal_id:?}): {message}")]
    OnChainDataUnserializable { internal_id: u64, message: String },
    #[error("Could not store the On Chain Data (snos job #{internal_id:?}): {message}")]
    OnChainDataUnstorable { internal_id: u64, message: String },
    #[error("Un-supported KZG flag")]
    UnsupportedKZGFlag,
}

impl SnosError {
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::CairoPieUnstorable { .. }
            | Self::SnosOutputUnstorable { .. }
            | Self::ProgramOutputUnstorable { .. }
            | Self::OnChainDataUnstorable { .. } => true,
            Self::SnosExecutionError { source, .. } => is_retryable_pie_generation_error(source),
            Self::UnspecifiedBlockNumber { .. }
            | Self::BlockNumberNotFound { .. }
            | Self::InvalidBlockNumber { .. }
            | Self::CairoPieUnserializable { .. }
            | Self::SnosOutputUnserializable { .. }
            | Self::ProgramOutputUnserializable { .. }
            | Self::FactCalculationError(_)
            | Self::Other(_)
            | Self::OnChainDataUnserializable { .. }
            | Self::UnsupportedKZGFlag => false,
        }
    }
}

fn is_retryable_pie_generation_error(error: &PieGenerationError) -> bool {
    match error {
        PieGenerationError::BlockProcessing { source, .. } => {
            source.downcast_ref::<BlockProcessingError>().is_some_and(is_retryable_block_processing_error)
        }
        PieGenerationError::RpcClient(message)
        | PieGenerationError::StateProcessing(message)
        | PieGenerationError::ContractClassProcessing(message) => is_retryable_remote_message(message),
        PieGenerationError::TaskJoin(_)
        | PieGenerationError::OsExecution(_)
        | PieGenerationError::Io(_)
        | PieGenerationError::InvalidConfig(_) => false,
    }
}

fn is_retryable_block_processing_error(error: &BlockProcessingError) -> bool {
    match error {
        BlockProcessingError::RpcClient(_) => true,
        BlockProcessingError::TransactionConversion { source, .. } => error_chain_is_retryable(source.as_ref()),
        BlockProcessingError::StateUpdate(source) => error_chain_is_retryable(source),
        BlockProcessingError::StorageProof(source) | BlockProcessingError::ClassProof(source) => {
            error_chain_is_retryable(source)
        }
        BlockProcessingError::StateUpdateProcessing(message)
        | BlockProcessingError::ContractClassConversion(message) => is_retryable_remote_message(message),
        BlockProcessingError::InvalidBlockState(message) => {
            let lower = message.to_ascii_lowercase();
            lower.contains("pending")
        }
        BlockProcessingError::InitialReadsExtension { source }
        | BlockProcessingError::InitialReadsSnapshot { source, .. }
        | BlockProcessingError::InitialReadClassHashHydration { source, .. }
        | BlockProcessingError::InitialReadNonceHydration { source, .. }
        | BlockProcessingError::InitialReadCompiledClassHashHydration { source, .. } => {
            error_chain_is_retryable(source)
        }
        BlockProcessingError::TransactionExecution(_)
        | BlockProcessingError::ContextBuilding(_)
        | BlockProcessingError::TransactionExecutorCreation { .. }
        | BlockProcessingError::StarknetVersion(_)
        | BlockProcessingError::MissingBlockStateAfterExecution
        | BlockProcessingError::InvalidContractAddress { .. }
        | BlockProcessingError::MissingProofField { .. }
        | BlockProcessingError::MissingProofFieldForContract { .. }
        | BlockProcessingError::InvalidOldBlockNumber { .. }
        | BlockProcessingError::Io(_)
        | BlockProcessingError::Serialization(_)
        | BlockProcessingError::Custom(_) => false,
    }
}

fn error_chain_is_retryable(error: &(dyn std::error::Error + 'static)) -> bool {
    if is_retryable_remote_message(&error.to_string()) {
        return true;
    }

    let mut source = error.source();
    while let Some(next) = source {
        if is_retryable_remote_message(&next.to_string()) {
            return true;
        }
        source = next.source();
    }

    false
}

fn is_retryable_remote_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();

    let non_retryable_markers = [
        "contractnotfound",
        "classhashnotfound",
        "undeclaredclasshash",
        "missing proof field",
        "invalid proof structure",
        "invalidtreeheight",
        "emptyproof",
        "invalidchildnodehash",
        "non-existence proof",
    ];
    if non_retryable_markers.iter().any(|marker| lower.contains(marker)) {
        return false;
    }

    let retryable_markers = [
        "rpcerror",
        "rpc error",
        "providererror",
        "ratelimited",
        "rate limited",
        "timeout",
        "timed out",
        "request error",
        "transport",
        "connection",
        "temporarily unavailable",
        "503",
        "429",
        "pendingblock",
        "still pending",
        "invalid response format",
        "proofconversionerror",
        "failed to convert rpc response",
    ];

    retryable_markers.iter().any(|marker| lower.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::*;
    use starknet::providers::ProviderError;

    #[test]
    fn snos_rpc_errors_are_retryable() {
        let error = SnosError::SnosExecutionError {
            internal_id: 7,
            source: PieGenerationError::BlockProcessing {
                block_number: 12,
                source: Box::new(BlockProcessingError::RpcClient(Box::new(ProviderError::RateLimited))),
            },
        };

        assert!(error.is_retryable());
    }

    #[test]
    fn snos_os_execution_errors_fail_fast() {
        let error = SnosError::SnosExecutionError {
            internal_id: 7,
            source: PieGenerationError::OsExecution("cairo execution failed".to_string()),
        };

        assert!(!error.is_retryable());
    }
}
