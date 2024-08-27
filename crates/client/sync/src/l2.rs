//! Contains the code required to sync data from the feeder efficiently.
use crate::fetch::fetchers::fetch_pending_block_and_updates;
use crate::fetch::l2_fetch_task;
use crate::metrics::block_metrics::BlockMetrics;
use crate::utils::trim_hash;
use anyhow::Context;
use dc_block_import::{
    BlockImportResult, BlockImporter, PreValidatedBlock, UnverifiedFullBlock, UnverifiedPendingFullBlock, Validation,
};
use dc_db::db_metrics::DbMetrics;
use dc_db::DeoxysBackend;
use dc_db::DeoxysStorageError;
use dc_telemetry::{TelemetryHandle, VerbosityLevel};
use dp_block::Header;
use dp_utils::{channel_wait_or_graceful_shutdown, stopwatch_end, wait_or_graceful_shutdown, PerfStopwatch};
use futures::{stream, StreamExt};
use num_traits::FromPrimitive;
use starknet_api::core::ChainId;
use starknet_providers::{ProviderError, SequencerGatewayProvider};
use starknet_types_core::felt::Felt;
use std::pin::pin;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinSet;
use tokio::time::Duration;

// TODO: add more explicit error variants
#[derive(thiserror::Error, Debug)]
pub enum L2SyncError {
    #[error("Provider error: {0:#}")]
    Provider(#[from] ProviderError),
    #[error("Database error: {0:#}")]
    Db(#[from] DeoxysStorageError),
    #[error(transparent)]
    BlockImport(#[from] dc_block_import::BlockImportError),
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
    backend: Arc<DeoxysBackend>,
    mut updates_receiver: mpsc::Receiver<PreValidatedBlock>,
    block_import: Arc<BlockImporter>,
    validation: Validation,
    backup_every_n_blocks: Option<u64>,
    block_metrics: BlockMetrics,
    db_metrics: DbMetrics,
    starting_block: u64,
    sync_timer: Arc<Mutex<Option<Instant>>>,
    telemetry: TelemetryHandle,
) -> anyhow::Result<()> {
    while let Some(block) = channel_wait_or_graceful_shutdown(pin!(updates_receiver.recv())).await {
        let BlockImportResult { header, block_hash } = block_import.verify_apply(block, validation.clone()).await?;

        update_sync_metrics(
            header.block_number,
            &header,
            starting_block,
            &block_metrics,
            &db_metrics,
            sync_timer.clone(),
            &backend,
        )
        .await?;

        let sw = PerfStopwatch::new();
        if backend.maybe_flush(false)? {
            stopwatch_end!(sw, "flush db: {:?}");
        }

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

    let sw = PerfStopwatch::new();
    if backend.maybe_flush(true)? {
        stopwatch_end!(sw, "flush db: {:?}");
    }

    Ok(())
}

async fn l2_block_conversion_task(
    updates_receiver: mpsc::Receiver<UnverifiedFullBlock>,
    output: mpsc::Sender<PreValidatedBlock>,
    block_import: Arc<BlockImporter>,
    validation: Validation,
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
    backend: Arc<DeoxysBackend>,
    block_import: Arc<BlockImporter>,
    validation: Validation,
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

        let block: UnverifiedPendingFullBlock =
            fetch_pending_block_and_updates(&backend, &provider).await.context("Getting pending block from FGW")?;

        let block = block_import.pre_validate_pending(block, validation.clone()).await?;
        block_import.verify_apply_pending(block, validation.clone()).await?;
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
}

/// Spawns workers to fetch blocks and state updates from the feeder.
#[allow(clippy::too_many_arguments)]
pub async fn sync(
    backend: &Arc<DeoxysBackend>,
    provider: SequencerGatewayProvider,
    config: L2SyncConfig,
    block_metrics: BlockMetrics,
    db_metrics: DbMetrics,
    starting_block: u64,
    chain_id: ChainId,
    telemetry: TelemetryHandle,
) -> anyhow::Result<()> {
    let (fetch_stream_sender, fetch_stream_receiver) = mpsc::channel(8);
    let (block_conv_sender, block_conv_receiver) = mpsc::channel(4);
    let provider = Arc::new(provider);
    let sync_timer = Arc::new(Mutex::new(None));
    let (once_caught_up_cb_sender, once_caught_up_cb_receiver) = oneshot::channel();

    // [Fetch task] ==new blocks and updates=> [Block conversion task] ======> [Verification and apply
    // task]
    // - Fetch task does parallel fetching
    // - Block conversion is compute heavy and parallel wrt. the next few blocks,
    // - Verification is sequential and does a lot of compute when state root verification is enabled.
    //   DB updates happen here too.

    // we are using separate tasks so that fetches don't get clogged up if by any chance the verify task
    // starves the tokio worker
    let block_importer = Arc::new(BlockImporter::new(Arc::clone(backend)));
    let validation = Validation { trust_transaction_hashes: false, trust_global_tries: config.verify, chain_id };

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
        block_metrics,
        db_metrics,
        starting_block,
        Arc::clone(&sync_timer),
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

async fn update_sync_metrics(
    block_number: u64,
    block_header: &Header,
    starting_block: u64,
    block_metrics: &BlockMetrics,
    db_metrics: &DbMetrics,
    sync_timer: Arc<Mutex<Option<Instant>>>,
    backend: &DeoxysBackend,
) -> anyhow::Result<()> {
    // Update Block sync time metrics
    let elapsed_time = {
        let mut timer_guard = sync_timer.lock().expect("Poisoned lock");
        *timer_guard = Some(Instant::now());
        if let Some(start_time) = *timer_guard {
            start_time.elapsed().as_secs_f64()
        } else {
            // For the first block, there is no previous timer set
            0.0
        }
    };

    let sync_time = block_metrics.l2_sync_time.get() + elapsed_time;
    block_metrics.l2_sync_time.set(sync_time);
    block_metrics.l2_latest_sync_time.set(elapsed_time);
    block_metrics.l2_avg_sync_time.set(block_metrics.l2_sync_time.get() / (block_number - starting_block) as f64);

    block_metrics.l2_block_number.set(block_header.block_number as f64);
    block_metrics.transaction_count.set(f64::from_u64(block_header.transaction_count).unwrap_or(0f64));
    block_metrics.event_count.set(f64::from_u64(block_header.event_count).unwrap_or(0f64));

    block_metrics.l1_gas_price_wei.set(f64::from_u128(block_header.l1_gas_price.eth_l1_gas_price).unwrap_or(0f64));
    block_metrics.l1_gas_price_strk.set(f64::from_u128(block_header.l1_gas_price.strk_l1_gas_price).unwrap_or(0f64));

    if block_number % 200 == 0 {
        let storage_size = backend.get_storage_size(db_metrics);
        let size_gb = storage_size as f64 / (1024 * 1024 * 1024) as f64;
        block_metrics.l2_state_size.set(size_gb);
    }

    Ok(())
}
