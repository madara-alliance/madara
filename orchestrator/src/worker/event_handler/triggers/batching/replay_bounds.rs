use starknet::core::types::{BlockId, MaybePreConfirmedBlockWithTxHashes};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider};
use std::sync::Arc;

#[derive(Debug)]
pub enum ReplayBoundsError {
    HashMismatch { block_number: u64, madara_hash: String, reference_hash: String },
    FetchFailed { block_number: u64, source: String, error: String },
    BlockPending { block_number: u64, source: String },
}

impl std::fmt::Display for ReplayBoundsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HashMismatch { block_number, madara_hash, reference_hash } => {
                write!(f, "block hash mismatch at #{block_number}: madara={madara_hash}, reference={reference_hash}")
            }
            Self::FetchFailed { block_number, source, error } => {
                write!(f, "failed to fetch block #{block_number} from {source}: {error}")
            }
            Self::BlockPending { block_number, source } => {
                write!(f, "block #{block_number} is still pending on {source}")
            }
        }
    }
}

fn extract_block_hash(
    result: Result<MaybePreConfirmedBlockWithTxHashes, impl std::fmt::Display>,
    block_number: u64,
    source: &str,
) -> Result<starknet::core::types::Felt, ReplayBoundsError> {
    match result {
        Ok(MaybePreConfirmedBlockWithTxHashes::Block(block)) => Ok(block.block_hash),
        Ok(MaybePreConfirmedBlockWithTxHashes::PreConfirmedBlock(_)) => {
            Err(ReplayBoundsError::BlockPending { block_number, source: source.into() })
        }
        Err(e) => Err(ReplayBoundsError::FetchFailed { block_number, source: source.into(), error: e.to_string() }),
    }
}

/// Validates that the block hash from the Madara (replay) node matches the reference node.
/// Fails closed: RPC errors and pending blocks halt batching rather than silently passing.
pub async fn validate_block_hash(
    madara_client: &Arc<JsonRpcClient<HttpTransport>>,
    reference_client: &Arc<JsonRpcClient<HttpTransport>>,
    block_number: u64,
) -> Result<(), ReplayBoundsError> {
    let block_id = BlockId::Number(block_number);

    let (madara_result, reference_result) = tokio::join!(
        madara_client.get_block_with_tx_hashes(block_id),
        reference_client.get_block_with_tx_hashes(block_id),
    );

    let madara_hash = extract_block_hash(madara_result, block_number, "madara")?;
    let reference_hash = extract_block_hash(reference_result, block_number, "reference")?;

    if madara_hash == reference_hash {
        tracing::debug!(block_number, hash = %madara_hash, "Replay bounds: block hash validated");
        Ok(())
    } else {
        Err(ReplayBoundsError::HashMismatch {
            block_number,
            madara_hash: format!("{madara_hash:#x}"),
            reference_hash: format!("{reference_hash:#x}"),
        })
    }
}
