use crate::error::job::fact::FactError;
use crate::error::other::OtherError;
use blockifier::blockifier::transaction_executor::TransactionExecutorError;
use blockifier::state::errors::StateError;
use blockifier::transaction::errors::{TransactionExecutionError, TransactionFeeError, TransactionPreValidationError};
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
        PieGenerationError::BlockProcessing { source, .. } => is_retryable_block_processing_error(source),
        PieGenerationError::RpcClient(message)
        | PieGenerationError::StateProcessing(message)
        | PieGenerationError::ContractClassProcessing(message) => is_retryable_remote_message(message),
        // A tokio JoinError means an internal SNOS task panicked or was cancelled. At this
        // boundary we cannot safely distinguish a deterministic bug from a transient RPC path
        // that panicked internally, so align it with the service-level panic policy and retry.
        PieGenerationError::TaskJoin(_) => true,
        PieGenerationError::OsExecution(_) | PieGenerationError::Io(_) | PieGenerationError::InvalidConfig(_) => false,
    }
}

fn is_retryable_block_processing_error(error: &BlockProcessingError) -> bool {
    match error {
        BlockProcessingError::RpcClient(source) => {
            error_chain_contains_block_not_found(source.as_ref()) || error_chain_is_retryable(source.as_ref())
        }
        BlockProcessingError::TransactionConversion { source, .. } => error_chain_is_retryable(source.as_ref()),
        BlockProcessingError::StateUpdate(source) => error_chain_is_retryable(source),
        BlockProcessingError::StorageProof(source) | BlockProcessingError::ClassProof(source) => {
            error_chain_is_retryable(source)
        }
        BlockProcessingError::StateUpdateProcessing(message)
        | BlockProcessingError::ContractClassConversion(message) => is_retryable_remote_message(message),
        BlockProcessingError::InvalidBlockState(message) => is_retryable_pending_block_state(message),
        BlockProcessingError::InitialReadsExtension { source }
        | BlockProcessingError::InitialReadsSnapshot { source, .. }
        | BlockProcessingError::InitialReadClassHashHydration { source, .. }
        | BlockProcessingError::InitialReadNonceHydration { source, .. }
        | BlockProcessingError::InitialReadCompiledClassHashHydration { source, .. } => {
            error_chain_is_retryable(source)
        }
        BlockProcessingError::TransactionExecution(source) => is_retryable_transaction_executor_error(source),
        BlockProcessingError::ContextBuilding(_)
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

fn error_chain_contains_block_not_found(error: &(dyn std::error::Error + 'static)) -> bool {
    if is_block_not_found_message(&error.to_string()) {
        return true;
    }

    let mut source = error.source();
    while let Some(next) = source {
        if is_block_not_found_message(&next.to_string()) {
            return true;
        }
        source = next.source();
    }

    false
}

fn is_retryable_transaction_executor_error(error: &TransactionExecutorError) -> bool {
    match error {
        TransactionExecutorError::StateError(source) => is_retryable_state_error(source),
        TransactionExecutorError::TransactionExecutionError(source) => is_retryable_transaction_execution_error(source),
        TransactionExecutorError::BlockFull | TransactionExecutorError::CompressionError(_) => false,
    }
}

fn is_retryable_transaction_execution_error(error: &TransactionExecutionError) -> bool {
    match error {
        TransactionExecutionError::StateError(source) => is_retryable_state_error(source),
        TransactionExecutionError::TransactionFeeError(source) => is_retryable_transaction_fee_error(source),
        TransactionExecutionError::TransactionPreValidationError(source) => {
            is_retryable_transaction_pre_validation_error(source)
        }
        _ => false,
    }
}

fn is_retryable_transaction_fee_error(error: &TransactionFeeError) -> bool {
    match error {
        TransactionFeeError::StateError(source) => is_retryable_state_error(source),
        _ => false,
    }
}

fn is_retryable_transaction_pre_validation_error(error: &TransactionPreValidationError) -> bool {
    match error {
        TransactionPreValidationError::StateError(source) => is_retryable_state_error(source),
        TransactionPreValidationError::TransactionFeeError(source) => is_retryable_transaction_fee_error(source),
        _ => false,
    }
}

fn is_retryable_state_error(error: &StateError) -> bool {
    match error {
        StateError::StateReadError(message) => is_retryable_state_read_message(message),
        _ => false,
    }
}

fn is_retryable_state_read_message(message: &str) -> bool {
    is_block_not_found_message(message)
}

fn is_block_not_found_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();

    lower.contains("blocknotfound") || lower.contains("block not found")
}

fn is_retryable_pending_block_state(message: &str) -> bool {
    matches!(
        message.to_ascii_lowercase().as_str(),
        "block is still pending"
            | "block receipts are still pending"
            | "previous block is still pending"
            | "older block is still pending"
    )
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
        "transport error",
        "transporterror",
        "connection reset",
        "connection refused",
        "connection aborted",
        "connection timed out",
        "error trying to connect",
        "dns error",
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
    use starknet::core::types::StarknetError;
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

    #[tokio::test]
    async fn snos_task_join_errors_are_retryable() {
        let join_error = tokio::spawn(async {
            panic!("simulated inner snos task panic");
        })
        .await
        .unwrap_err();

        let error = SnosError::SnosExecutionError { internal_id: 7, source: PieGenerationError::TaskJoin(join_error) };

        assert!(error.is_retryable());
    }

    #[test]
    fn permanent_rpc_errors_fail_fast() {
        let error = SnosError::SnosExecutionError {
            internal_id: 7,
            source: PieGenerationError::BlockProcessing {
                block_number: 12,
                source: Box::new(BlockProcessingError::RpcClient(Box::new(ProviderError::StarknetError(
                    StarknetError::ContractNotFound,
                )))),
            },
        };

        assert!(!error.is_retryable());
    }

    #[test]
    fn transaction_execution_state_read_block_not_found_is_retryable() {
        let error = SnosError::SnosExecutionError {
            internal_id: 7,
            source: PieGenerationError::BlockProcessing {
                block_number: 12,
                source: Box::new(BlockProcessingError::TransactionExecution(TransactionExecutorError::StateError(
                    StateError::StateReadError("BlockNotFound".to_string()),
                ))),
            },
        };

        assert!(error.is_retryable());
    }

    #[test]
    fn rpc_client_block_not_found_is_retryable() {
        let error = SnosError::SnosExecutionError {
            internal_id: 7,
            source: PieGenerationError::BlockProcessing {
                block_number: 12,
                source: Box::new(BlockProcessingError::RpcClient(Box::new(ProviderError::StarknetError(
                    StarknetError::BlockNotFound,
                )))),
            },
        };

        assert!(error.is_retryable());
    }

    #[test]
    fn exact_rpc_client_block_not_found_error_string_is_retryable() {
        let error = crate::error::job::JobError::from(SnosError::SnosExecutionError {
            internal_id: 646856,
            source: PieGenerationError::BlockProcessing {
                block_number: 2222689,
                source: Box::new(BlockProcessingError::RpcClient(Box::new(ProviderError::StarknetError(
                    StarknetError::BlockNotFound,
                )))),
            },
        });

        assert_eq!(
            error.to_string(),
            "Snos Error: Error while running SNOS (snos job #646856): Block processing failed for block 2222689: \
             RPC client error: BlockNotFound"
        );
        assert!(!error.is_fail_fast_snos_processing_failure());
    }

    #[test]
    fn exact_block_not_found_error_string_is_retryable() {
        let error = SnosError::SnosExecutionError {
            internal_id: 645003,
            source: PieGenerationError::BlockProcessing {
                block_number: 2216682,
                source: Box::new(BlockProcessingError::TransactionExecution(TransactionExecutorError::StateError(
                    StateError::StateReadError("BlockNotFound".to_string()),
                ))),
            },
        };

        assert_eq!(
            error.to_string(),
            "Error while running SNOS (snos job #645003): Block processing failed for block 2216682: \
             Transaction execution error: Failed to read from state: BlockNotFound."
        );
        assert!(error.is_retryable());
    }

    #[test]
    fn transaction_execution_non_block_not_found_state_reads_fail_fast() {
        let error = SnosError::SnosExecutionError {
            internal_id: 7,
            source: PieGenerationError::BlockProcessing {
                block_number: 12,
                source: Box::new(BlockProcessingError::TransactionExecution(TransactionExecutorError::StateError(
                    StateError::StateReadError("Failed to decompress legacy contract class".to_string()),
                ))),
            },
        };

        assert!(!error.is_retryable());
    }

    #[test]
    fn transaction_execution_state_read_rpc_errors_do_not_retry_via_block_not_found_rule() {
        let error = SnosError::SnosExecutionError {
            internal_id: 7,
            source: PieGenerationError::BlockProcessing {
                block_number: 12,
                source: Box::new(BlockProcessingError::TransactionExecution(TransactionExecutorError::StateError(
                    StateError::StateReadError("RPC error: timeout".to_string()),
                ))),
            },
        };

        assert!(!error.is_retryable());
    }

    #[test]
    fn only_known_pending_block_state_errors_are_retryable() {
        assert!(is_retryable_pending_block_state("Block is still pending"));
        assert!(is_retryable_pending_block_state("Block receipts are still pending"));
        assert!(!is_retryable_pending_block_state("Cannot convert pending block: missing header"));
    }

    #[test]
    fn broad_connection_substrings_no_longer_trigger_retries() {
        assert!(!is_retryable_remote_message("No active connection for contract"));
        assert!(is_retryable_remote_message("Encountered a request error: connection reset by peer"));
        assert!(is_retryable_remote_message("Provider error: transport error"));
    }
}
