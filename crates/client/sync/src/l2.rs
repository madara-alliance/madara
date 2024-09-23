//! Contains the code required to sync data from the feeder efficiently.
use crate::fetch::fetchers::fetch_pending_block_and_updates;
use crate::fetch::l2_fetch_task;
use crate::utils::trim_hash;
use anyhow::Context;
use futures::{stream, StreamExt};
use mc_block_import::{
    BlockImportResult, BlockImporter, BlockValidationContext, PreValidatedBlock, UnverifiedFullBlock,
};
use mc_db::MadaraBackend;
use mc_db::MadaraStorageError;
use mc_telemetry::{TelemetryHandle, VerbosityLevel};
use mp_utils::{channel_wait_or_graceful_shutdown, wait_or_graceful_shutdown, PerfStopwatch};
use starknet_api::core::ChainId;
use starknet_providers::{ProviderError, SequencerGatewayProvider};
use starknet_types_core::felt::Felt;
use std::pin::pin;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinSet;
use tokio::time::Duration;

// TODO: add more explicit error variants
#[derive(thiserror::Error, Debug)]
pub enum L2SyncError {
    #[error("Provider error: {0:#}")]
    Provider(#[from] ProviderError),
    #[error("Database error: {0:#}")]
    Db(#[from] MadaraStorageError),
    #[error(transparent)]
    BlockImport(#[from] mc_block_import::BlockImportError),
    #[error("Unexpected class type for class hash {class_hash:#x}")]
    UnexpectedClassType { class_hash: Felt },
}

/// Contains the latest Starknet verified state on L2
#[derive(Debug, Clone)]
pub struct L2StateUpdate {
    pub block_number: u64,
    pub global_root: Felt,
    pub block_hash: Felt,
}

#[allow(clippy::too_many_arguments)]
async fn l2_verify_and_apply_task(
    backend: Arc<MadaraBackend>,
    mut updates_receiver: mpsc::Receiver<PreValidatedBlock>,
    block_import: Arc<BlockImporter>,
    validation: BlockValidationContext,
    backup_every_n_blocks: Option<u64>,
    telemetry: TelemetryHandle,
) -> anyhow::Result<()> {
    while let Some(block) = channel_wait_or_graceful_shutdown(pin!(updates_receiver.recv())).await {
        let BlockImportResult { header, block_hash } = block_import.verify_apply(block, validation.clone()).await?;

        log::info!(
            "✨ Imported #{} ({}) and updated state root ({})",
            header.block_number,
            trim_hash(&block_hash),
            trim_hash(&header.global_state_root)
        );
        log::debug!(
            "Block import #{} ({}) has state root {}",
            header.block_number,
            block_hash,
            header.global_state_root
        );

        telemetry.send(
            VerbosityLevel::Info,
            serde_json::json!({
                "best": block_hash.to_fixed_hex_string(),
                "height": header.block_number,
                "origin": "Own",
                "msg": "block.import",
            }),
        );

        if backup_every_n_blocks.is_some_and(|backup_every_n_blocks| header.block_number % backup_every_n_blocks == 0) {
            log::info!("⏳ Backing up database at block {}...", header.block_number);
            let sw = PerfStopwatch::new();
            backend.backup().await.context("backing up database")?;
            log::info!("✅ Database backup is done ({:?})", sw.elapsed());
        }
    }

    Ok(())
}

async fn l2_block_conversion_task(
    updates_receiver: mpsc::Receiver<UnverifiedFullBlock>,
    output: mpsc::Sender<PreValidatedBlock>,
    block_import: Arc<BlockImporter>,
    validation: BlockValidationContext,
) -> anyhow::Result<()> {
    // Items of this stream are futures that resolve to blocks, which becomes a regular stream of blocks
    // using futures buffered.
    let conversion_stream = stream::unfold(
        (updates_receiver, block_import, validation.clone()),
        |(mut updates_recv, block_import, validation)| async move {
            channel_wait_or_graceful_shutdown(updates_recv.recv()).await.map(|block| {
                let block_import_ = Arc::clone(&block_import);
                let validation_ = validation.clone();
                (
                    async move { block_import_.pre_validate(block, validation_).await },
                    (updates_recv, block_import, validation),
                )
            })
        },
    );

    let mut stream = pin!(conversion_stream.buffered(10));
    while let Some(block) = channel_wait_or_graceful_shutdown(stream.next()).await {
        if output.send(block?).await.is_err() {
            // channel closed
            break;
        }
    }
    Ok(())
}

async fn l2_pending_block_task(
    backend: Arc<MadaraBackend>,
    block_import: Arc<BlockImporter>,
    validation: BlockValidationContext,
    sync_finished_cb: oneshot::Receiver<()>,
    provider: Arc<SequencerGatewayProvider>,
    pending_block_poll_interval: Duration,
) -> anyhow::Result<()> {
    // clear pending status
    {
        backend.clear_pending_block().context("Clearing pending block")?;
        log::debug!("l2_pending_block_task: startup: wrote no pending");
    }

    // we start the pending block task only once the node has been fully sync
    if sync_finished_cb.await.is_err() {
        // channel closed
        return Ok(());
    }

    log::debug!("start pending block poll");

    let mut interval = tokio::time::interval(pending_block_poll_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    while wait_or_graceful_shutdown(interval.tick()).await.is_some() {
        log::debug!("getting pending block...");

        let block = fetch_pending_block_and_updates(&backend.chain_config().chain_id, &provider)
            .await
            .context("Getting pending block from FGW")?;

        // HACK(see issue #239): The latest block in db may not match the pending parent block hash
        // Just silently ignore it for now and move along.
        let import_block = || async {
            let block = block_import.pre_validate_pending(block, validation.clone()).await?;
            block_import.verify_apply_pending(block, validation.clone()).await?;
            anyhow::Ok(())
        };

        if let Err(err) = import_block().await {
            log::debug!("Error while importing pending block: {err:#}");
        }
    }

    Ok(())
}

pub struct L2SyncConfig {
    pub first_block: u64,
    pub n_blocks_to_sync: Option<u64>,
    pub verify: bool,
    pub sync_polling_interval: Option<Duration>,
    pub backup_every_n_blocks: Option<u64>,
    pub pending_block_poll_interval: Duration,
    pub ignore_block_order: bool,
}

/// Spawns workers to fetch blocks and state updates from the feeder.
#[allow(clippy::too_many_arguments)]
pub async fn sync(
    backend: &Arc<MadaraBackend>,
    provider: SequencerGatewayProvider,
    config: L2SyncConfig,
    chain_id: ChainId,
    telemetry: TelemetryHandle,
    block_importer: Arc<BlockImporter>,
) -> anyhow::Result<()> {
    let (fetch_stream_sender, fetch_stream_receiver) = mpsc::channel(8);
    let (block_conv_sender, block_conv_receiver) = mpsc::channel(4);
    let provider = Arc::new(provider);
    let (once_caught_up_cb_sender, once_caught_up_cb_receiver) = oneshot::channel();

    // [Fetch task] ==new blocks and updates=> [Block conversion task] ======> [Verification and apply
    // task]
    // - Fetch task does parallel fetching
    // - Block conversion is compute heavy and parallel wrt. the next few blocks,
    // - Verification is sequential and does a lot of compute when state root verification is enabled.
    //   DB updates happen here too.

    // we are using separate tasks so that fetches don't get clogged up if by any chance the verify task
    // starves the tokio worker
    let validation = BlockValidationContext {
        trust_transaction_hashes: false,
        trust_global_tries: config.verify,
        chain_id,
        trust_class_hashes: false,
        ignore_block_order: config.ignore_block_order,
    };

    let mut join_set = JoinSet::new();
    join_set.spawn(l2_fetch_task(
        Arc::clone(backend),
        config.first_block,
        config.n_blocks_to_sync,
        fetch_stream_sender,
        Arc::clone(&provider),
        config.sync_polling_interval,
        once_caught_up_cb_sender,
    ));
    join_set.spawn(l2_block_conversion_task(
        fetch_stream_receiver,
        block_conv_sender,
        Arc::clone(&block_importer),
        validation.clone(),
    ));
    join_set.spawn(l2_verify_and_apply_task(
        Arc::clone(backend),
        block_conv_receiver,
        Arc::clone(&block_importer),
        validation.clone(),
        config.backup_every_n_blocks,
        telemetry,
    ));
    join_set.spawn(l2_pending_block_task(
        Arc::clone(backend),
        Arc::clone(&block_importer),
        validation.clone(),
        once_caught_up_cb_receiver,
        provider,
        config.pending_block_poll_interval,
    ));

    while let Some(res) = join_set.join_next().await {
        res.context("task was dropped")??;
    }

    Ok(())
}
