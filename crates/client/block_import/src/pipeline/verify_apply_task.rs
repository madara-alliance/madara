use dp_utils::channel_wait_or_graceful_shutdown;
use starknet_core::types::Felt;
use tokio::sync::mpsc;

use crate::{PipelineSettings, PreValidatedBlock, Validation, VerifyApply, VerifyApplyResult};

pub fn trim_hash(hash: &Felt) -> String {
    let hash_str = format!("{:#x}", hash);
    let hash_len = hash_str.len();

    let prefix = &hash_str[..6 + 2];
    let suffix = &hash_str[hash_len - 6..];

    format!("{}...{}", prefix, suffix)
}

pub async fn verify_apply_task(
    validation: &Validation,
    pipeline_settings: &PipelineSettings,
    verify_apply: VerifyApply,
    mut input: mpsc::Receiver<PreValidatedBlock>,
) -> anyhow::Result<()> {
    while let Some(block) = channel_wait_or_graceful_shutdown(input.recv()).await {
        match verify_apply.verify_apply(block, validation).await {
            Ok(VerifyApplyResult { block_hash, block_number, state_root }) => {
                log::info!(
                    "✨ Imported #{} ({}) and updated state root ({})",
                    block_number,
                    trim_hash(&block_hash),
                    trim_hash(&state_root)
                );
                log::debug!("Block import #{} ({:#x}) with root {:#x}", block_number, block_hash, state_root);
            }
            Err(_) => todo!(),
        }
    }

    Ok(())
}
