use starknet::core::types::{BlockId, MaybePreConfirmedBlockWithTxHashes};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider};
use std::sync::Arc;

#[derive(Debug)]
pub struct BlockHashMismatch {
    pub block_number: u64,
    pub madara_hash: String,
    pub reference_hash: String,
}

impl std::fmt::Display for BlockHashMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Replay bounds: block hash mismatch at block #{}: madara={}, reference={}",
            self.block_number, self.madara_hash, self.reference_hash
        )
    }
}

/// Validates that the block hash from the Madara (replay) node matches the reference node.
/// Returns Ok(()) on match, Err(BlockHashMismatch) on mismatch.
pub async fn validate_block_hash(
    madara_client: &Arc<JsonRpcClient<HttpTransport>>,
    reference_client: &Arc<JsonRpcClient<HttpTransport>>,
    block_number: u64,
) -> Result<(), BlockHashMismatch> {
    let block_id = BlockId::Number(block_number);

    let (madara_result, reference_result) = tokio::join!(
        madara_client.get_block_with_tx_hashes(block_id),
        reference_client.get_block_with_tx_hashes(block_id),
    );

    let madara_hash = match madara_result {
        Ok(MaybePreConfirmedBlockWithTxHashes::Block(block)) => block.block_hash,
        Ok(MaybePreConfirmedBlockWithTxHashes::PreConfirmedBlock(_)) => {
            tracing::warn!(block_number, "Replay bounds: block is still pending on Madara node");
            return Ok(());
        }
        Err(e) => {
            tracing::error!(block_number, error = %e, "Replay bounds: failed to fetch block from Madara node");
            return Ok(());
        }
    };

    let reference_hash = match reference_result {
        Ok(MaybePreConfirmedBlockWithTxHashes::Block(block)) => block.block_hash,
        Ok(MaybePreConfirmedBlockWithTxHashes::PreConfirmedBlock(_)) => {
            tracing::warn!(block_number, "Replay bounds: block is still pending on reference node");
            return Ok(());
        }
        Err(e) => {
            tracing::error!(block_number, error = %e, "Replay bounds: failed to fetch block from reference node");
            return Ok(());
        }
    };

    if madara_hash == reference_hash {
        tracing::debug!(block_number, hash = %madara_hash, "Replay bounds: block hash validated");
        Ok(())
    } else {
        Err(BlockHashMismatch {
            block_number,
            madara_hash: format!("{madara_hash:#x}"),
            reference_hash: format!("{reference_hash:#x}"),
        })
    }
}
