//! Contains the code required to sync data from the feeder efficiently.
use crate::commitments::update_tries_and_compute_state_root;
use crate::convert::{convert_and_verify_block, convert_and_verify_class};
use crate::fetch::fetchers::{fetch_block_and_updates, FetchBlockId, L2BlockAndUpdates};
use crate::fetch::l2_fetch_task;
use crate::metrics::block_metrics::BlockMetrics;
use crate::utility::trim_hash;
use anyhow::{bail, Context};
use mc_db::db_metrics::DbMetrics;
use mc_db::MadaraBackend;
use mc_db::MadaraStorageError;
use mc_telemetry::{TelemetryHandle, VerbosityLevel};
use mp_block::{BlockId, BlockTag, MadaraBlock, MadaraMaybePendingBlockInfo, StarknetVersionError};
use mp_block::{Header, MadaraMaybePendingBlock};
use mp_class::ConvertedClass;
use mp_state_update::StateDiff;
use mp_transactions::TransactionTypeError;
use mp_utils::{
    channel_wait_or_graceful_shutdown, spawn_rayon_task, stopwatch_end, wait_or_graceful_shutdown, PerfStopwatch,
};
use futures::{stream, StreamExt};
use num_traits::FromPrimitive;
use starknet_providers::{ProviderError, SequencerGatewayProvider};
use starknet_types_core::felt::Felt;
use std::borrow::Cow;
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
    Db(#[from] MadaraStorageError),
    #[error("Malformated block: {0}")]
    BlockFormat(Cow<'static, str>),
    #[error("Mismatched block hash for block {0}")]
    MismatchedBlockHash(u64),
    #[error("Gas price is too high: 0x{0:x}")]
    GasPriceOutOfBounds(Felt),
    #[error("Invalid Starknet version: {0}")]
    InvalidStarknetVersion(#[from] StarknetVersionError),
    #[error("Invalid transaction: {0}")]
    InvalidTransaction(#[from] TransactionTypeError),
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
    mut updates_receiver: mpsc::Receiver<L2ConvertedBlockAndUpdates>,
    verify: bool,
    backup_every_n_blocks: Option<u64>,
    block_metrics: BlockMetrics,
    db_metrics: DbMetrics,
    starting_block: u64,
    sync_timer: Arc<Mutex<Option<Instant>>>,
    telemetry: TelemetryHandle,
) -> anyhow::Result<()> {
    while let Some(L2ConvertedBlockAndUpdates { converted_block, converted_state_diff, converted_classes }) =
        channel_wait_or_graceful_shutdown(pin!(updates_receiver.recv())).await
    {
        let block_n = converted_block.info.header.block_number;
        let block_hash = converted_block.info.block_hash;
        let global_state_root = converted_block.info.header.global_state_root;

        let state_diff = if verify {
            let state_diff = Arc::new(converted_state_diff);
            let state_diff_1 = Arc::clone(&state_diff);
            let backend = Arc::clone(&backend);

            let state_root = spawn_rayon_task(move || {
                let sw = PerfStopwatch::new();
                let state_root = update_tries_and_compute_state_root(&backend, &state_diff, block_n);
                stopwatch_end!(sw, "verify_l2: {:?}");

                anyhow::Ok(state_root)
            })
            .await?;

            if global_state_root != state_root {
                bail!(
                    "Verified state root: {:#x} doesn't match fetched state root: {:#x}",
                    state_root,
                    global_state_root
                );
            }

            // UNWRAP: we need a 'static future as we are spawning tokio tasks further down the line
            //         this is a hack to achieve that, we put the update in an arc and then unwrap it at the end
            //         this will not panic as the Arc should not be aliased.
            Arc::try_unwrap(state_diff_1).unwrap()
        } else {
            converted_state_diff
        };

        let block_header = converted_block.info.header.clone();
        let backend_ = Arc::clone(&backend);
        spawn_rayon_task(move || {
            backend_
                .store_block(
                    MadaraMaybePendingBlock {
                        info: MadaraMaybePendingBlockInfo::NotPending(converted_block.info),
                        inner: converted_block.inner,
                    },
                    state_diff,
                    converted_classes,
                )
                .context("Storing new block")?;

            anyhow::Ok(())
        })
        .await?;

        update_sync_metrics(
            block_n,
            &block_header,
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
            block_n,
            trim_hash(&block_hash),
            trim_hash(&global_state_root)
        );
        log::debug!("Imported #{} ({}) and updated state root ({})", block_n, block_hash, global_state_root);

        telemetry.send(
            VerbosityLevel::Info,
            serde_json::json!({
                "best": block_hash.to_fixed_hex_string(),
                "height": block_n,
                "origin": "Own",
                "msg": "block.import",
            }),
        );

        if backup_every_n_blocks.is_some_and(|backup_every_n_blocks| block_n % backup_every_n_blocks == 0) {
            log::info!("⏳ Backing up database at block {block_n}...");
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

pub struct L2ConvertedBlockAndUpdates {
    pub converted_block: MadaraBlock,
    pub converted_state_diff: StateDiff,
    pub converted_classes: Vec<ConvertedClass>,
}

async fn l2_block_conversion_task(
    updates_receiver: mpsc::Receiver<L2BlockAndUpdates>,
    output: mpsc::Sender<L2ConvertedBlockAndUpdates>,
    chain_id: Felt,
) -> anyhow::Result<()> {
    // Items of this stream are futures that resolve to blocks, which becomes a regular stream of blocks
    // using futures buffered.
    let conversion_stream = stream::unfold((updates_receiver, chain_id), |(mut updates_recv, chain_id)| async move {
        channel_wait_or_graceful_shutdown(updates_recv.recv()).await.map(
            |L2BlockAndUpdates { block, state_diff, class_update, .. }| {
                (
                    spawn_rayon_task(move || {
                        let sw = PerfStopwatch::new();
                        let block_n = block.block_number;
                        let task_convert_block =
                            || convert_and_verify_block(block, state_diff, chain_id).context("Converting block");
                        let task_convert_classes =
                            || convert_and_verify_class(class_update, block_n).context("Converting classes");
                        let (converted_block_with_state_diff, converted_classes) =
                            rayon::join(task_convert_block, task_convert_classes);
                        stopwatch_end!(sw, "convert_block_and_class {:?}: {:?}", block_n);
                        let (converted_block, converted_state_diff) = converted_block_with_state_diff?;
                        anyhow::Ok(L2ConvertedBlockAndUpdates {
                            converted_block,
                            converted_state_diff,
                            converted_classes: converted_classes?,
                        })
                    }),
                    (updates_recv, chain_id),
                )
            },
        )
    });

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
    sync_finished_cb: oneshot::Receiver<()>,
    provider: Arc<SequencerGatewayProvider>,
    chain_id: Felt,
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

        let L2BlockAndUpdates { block_id: _, block, state_diff, class_update } =
            fetch_block_and_updates(&backend, FetchBlockId::Pending, &provider)
                .await
                .context("Getting pending block from sequencer")?;

        let block_hash_best = backend
            .get_block_hash(&BlockId::Tag(BlockTag::Latest))
            .context("Getting latest block in db")?
            .context("No block in db")?;

        log::debug!("pending block hash parent hash: {:#x}", block.parent_block_hash);

        if block.parent_block_hash == block_hash_best {
            log::debug!("pending block parent block hash matches chain tip, writing pending block");

            let backend_ = Arc::clone(&backend);
            spawn_rayon_task(move || {
                let (block, converted_state_diff) =
                    crate::convert::convert_pending(block, state_diff, chain_id).context("Converting pending block")?;
                let convert_classes = convert_and_verify_class(class_update, None).context("Converting classes")?;

                backend_
                    .store_block(
                        MadaraMaybePendingBlock {
                            info: MadaraMaybePendingBlockInfo::Pending(block.info),
                            inner: block.inner,
                        },
                        converted_state_diff,
                        convert_classes,
                    )
                    .context("Storing new block")?;

                anyhow::Ok(())
            })
            .await?;
        } else {
            log::debug!("pending block parent hash does not match latest block, clearing pending block");
            backend.clear_pending_block().context("Clearing pending block")?;
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
}

/// Spawns workers to fetch blocks and state updates from the feeder.
#[allow(clippy::too_many_arguments)]
pub async fn sync(
    backend: &Arc<MadaraBackend>,
    provider: SequencerGatewayProvider,
    config: L2SyncConfig,
    block_metrics: BlockMetrics,
    db_metrics: DbMetrics,
    starting_block: u64,
    chain_id: Felt,
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
    join_set.spawn(l2_block_conversion_task(fetch_stream_receiver, block_conv_sender, chain_id));
    join_set.spawn(l2_verify_and_apply_task(
        Arc::clone(backend),
        block_conv_receiver,
        config.verify,
        config.backup_every_n_blocks,
        block_metrics,
        db_metrics,
        starting_block,
        Arc::clone(&sync_timer),
        telemetry,
    ));
    join_set.spawn(l2_pending_block_task(
        Arc::clone(backend),
        once_caught_up_cb_receiver,
        provider,
        chain_id,
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
    backend: &MadaraBackend,
) -> anyhow::Result<()> {
    // Update Block sync time metrics
    let elapsed_time = {
        let mut timer_guard = sync_timer.lock().unwrap();
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
