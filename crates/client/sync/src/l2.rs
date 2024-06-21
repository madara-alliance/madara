//! Contains the code required to sync data from the feeder efficiently.
use std::pin::pin;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{bail, Context};
use dc_db::mapping_db::MappingDbError;
use dc_db::rocksdb::{WriteBatchWithTransaction, WriteOptions};
use dc_db::storage_handler::primitives::contract_class::{ClassUpdateWrapper, ContractClassData};
use dc_db::storage_handler::DeoxysStorageError;
use dc_db::storage_updates::{store_class_update, store_key_update, store_state_update};
use dc_db::DeoxysBackend;
use dc_telemetry::{TelemetryHandle, VerbosityLevel};
use dp_block::Header;
use dp_block::{BlockId, BlockTag, DeoxysBlock};
use dp_convert::ToFelt;
use dp_convert::ToStarkFelt;
use futures::{stream, StreamExt};
use num_traits::FromPrimitive;
use starknet_core::types::StateUpdate;
use starknet_providers::sequencer::models::StateUpdateWithBlock;
use starknet_providers::{ProviderError, SequencerGatewayProvider};
use starknet_types_core::felt::Felt;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinSet;
use tokio::time::Duration;

use crate::commitments::{build_commitment_state_diff, csd_calculate_state_root};
use crate::convert::convert_block;
use crate::fetch::fetchers::L2BlockAndUpdates;
use crate::fetch::l2_fetch_task;
use crate::metrics::block_metrics::BlockMetrics;
use crate::utility::trim_hash;
use dp_utils::{
    channel_wait_or_graceful_shutdown, spawn_compute, stopwatch_end, wait_or_graceful_shutdown, PerfStopwatch,
};

// TODO: add more error variants, which are more explicit
#[derive(thiserror::Error, Debug)]
pub enum L2SyncError {
    #[error("provider error")]
    Provider(#[from] ProviderError),
    #[error("db error")]
    Db(#[from] DeoxysStorageError),
    #[error("mismatched block hash for block {0}")]
    MismatchedBlockHash(u64),
}

/// Contains the latest Starknet verified state on L2
#[derive(Debug, Clone)]
pub struct L2StateUpdate {
    pub block_number: u64,
    pub global_root: Felt,
    pub block_hash: Felt,
}

fn store_new_block(backend: &DeoxysBackend, block: &DeoxysBlock) -> Result<(), DeoxysStorageError> {
    let sw = PerfStopwatch::new();
    let mut tx = WriteBatchWithTransaction::default();

    let mapping = backend.mapping();
    mapping.write_new_block(&mut tx, block)?;

    let db_access = backend.expose_db();
    let mut write_opt = WriteOptions::default(); // todo move that in db
    write_opt.disable_wal(true);
    db_access.write_opt(tx, &write_opt).map_err(MappingDbError::from)?;
    log::debug!("committed store_new_block");

    stopwatch_end!(sw, "end store_new_block {}: {:?}", block.block_n());
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn l2_verify_and_apply_task(
    backend: Arc<DeoxysBackend>,
    mut updates_receiver: mpsc::Receiver<L2ConvertedBlockAndUpdates>,
    verify: bool,
    backup_every_n_blocks: Option<u64>,
    block_metrics: BlockMetrics,
    starting_block: u64,
    sync_timer: Arc<Mutex<Option<Instant>>>,
    telemetry: TelemetryHandle,
) -> anyhow::Result<()> {
    while let Some(L2ConvertedBlockAndUpdates { converted_block, state_update, class_update }) =
        channel_wait_or_graceful_shutdown(pin!(updates_receiver.recv())).await
    {
        let block_n = converted_block.block_n();
        let block_hash = *converted_block.block_hash();
        let global_state_root = converted_block.header().global_state_root;

        let state_update = if verify {
            let state_update = Arc::new(state_update);
            let state_update_1 = Arc::clone(&state_update);
            let backend = Arc::clone(&backend);

            let state_root = spawn_compute(move || {
                let sw = PerfStopwatch::new();
                let state_root = verify_l2(&backend, block_n, &state_update)?;
                stopwatch_end!(sw, "verify_l2: {:?}");

                anyhow::Ok(state_root)
            })
            .await?;

            if global_state_root.to_felt() != state_root {
                // TODO(fault tolerance): we should have a single rocksdb transaction for the whole l2 update.
                // let prev_block = block_n.checked_sub(1).expect("no block to revert to");

                // storage_handler::contract_trie_mut().revert_to(prev_block);
                // storage_handler::contract_storage_trie_mut().revert_to(prev_block);
                // storage_handler::contract_class_trie_mut().revert_to(prev_block);
                // TODO(charpa): make other stuff revertible, maybe history?

                bail!("Verified state root: {} doesn't match fetched state root: {}", state_root, global_state_root);
            }

            // UNWRAP: we need a 'static future as we are spawning tokio tasks further down the line
            //         this is a hack to achieve that, we put the update in an arc and then unwrap it at the end
            //         this will not panic as the Arc should not be aliased.
            Arc::try_unwrap(state_update_1).unwrap()
        } else {
            state_update
        };

        let storage_diffs = state_update.state_diff.storage_diffs.clone();

        let (r1, (r2, (r3, r4))) = rayon::join(
            || store_new_block(&backend, &converted_block),
            || {
                rayon::join(
                    || store_state_update(&backend, block_n, state_update),
                    || {
                        rayon::join(
                            || store_class_update(&backend, block_n, ClassUpdateWrapper(class_update)),
                            || store_key_update(&backend, block_n, &storage_diffs),
                        )
                    },
                )
            },
        );
        r1.and(r2).and(r3).and(r4).context("storing new block")?;

        update_sync_metrics(
            // &mut command_sink,
            block_n,
            converted_block.header(),
            starting_block,
            &block_metrics,
            sync_timer.clone(),
        )
        .await?;

        let sw = PerfStopwatch::new();
        backend.maybe_flush()?;
        stopwatch_end!(sw, "flush db: {:?}");

        log::info!(
            "✨ Imported #{} ({}) and updated state root ({})",
            block_n,
            trim_hash(&block_hash.to_felt()),
            trim_hash(&global_state_root.to_felt())
        );
        log::debug!("Imported #{} ({}) and updated state root ({})", block_n, block_hash.0, global_state_root);

        telemetry.send(
            VerbosityLevel::Info,
            serde_json::json!({
                "best": format!("{}", block_hash.0),
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

    Ok(())
}

pub struct L2ConvertedBlockAndUpdates {
    pub converted_block: DeoxysBlock,
    pub state_update: StateUpdate,
    pub class_update: Vec<ContractClassData>,
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
            |L2BlockAndUpdates { block, state_update, class_update, .. }| {
                (
                    spawn_compute(move || {
                        let sw = PerfStopwatch::new();
                        let block_n = block.block_number;
                        let converted_block = convert_block(block, chain_id).context("converting block")?;
                        stopwatch_end!(sw, "convert_block {:?}: {:?}", block_n);
                        anyhow::Ok(L2ConvertedBlockAndUpdates {
                            converted_block: converted_block,
                            state_update,
                            class_update,
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
    backend: Arc<DeoxysBackend>,
    sync_finished_cb: oneshot::Receiver<()>,
    provider: Arc<SequencerGatewayProvider>,
    chain_id: Felt,
    pending_block_poll_interval: Duration,
) -> anyhow::Result<()> {
    let backend = &backend;

    // clear pending status
    {
        let mut tx = WriteBatchWithTransaction::default();
        backend.mapping().write_no_pending(&mut tx).context("clearing pending status")?;
        let mut write_opt = WriteOptions::default(); // todo move that in db
        write_opt.disable_wal(true);
        backend.expose_db().write_opt(tx, &write_opt).context("clearing pending block to db")?;
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
        let storage = backend.mapping();

        log::debug!("getting pending block...");

        let StateUpdateWithBlock { state_update, block } = provider
            .get_state_update_with_block(starknet_providers::sequencer::models::BlockId::Pending)
            .await
            .context("getting pending block from sequencer")?;

        let block_hash_best = storage
            .get_block_hash(&BlockId::Tag(BlockTag::Latest))
            .context("getting latest block in db")?
            .context("no block in db")?;

        log::debug!("pending block hash parent hash: {:#}", block.parent_block_hash.to_stark_felt());

        let mut tx = WriteBatchWithTransaction::default();
        if block.parent_block_hash == block_hash_best.to_felt() {
            let block = spawn_compute(move || crate::convert::convert_block(block, chain_id))
                .await
                .context("converting pending block")?;

            log::debug!("pending block parent block hash matches chain tip, writing pending block");

            let storage_update = crate::convert::state_update(state_update);
            storage
                .write_pending(&mut tx, &block, &storage_update)
                .context("writing pending to rocksdb transaction")?;
        } else {
            log::debug!("pending block parent hash does not match latest block, clearing pending block");
            storage.write_no_pending(&mut tx).context("writing no pending to rocksdb transaction")?;
        }

        // todo move that in db somehow
        let mut write_opt = WriteOptions::default();
        write_opt.disable_wal(true);
        backend.expose_db().write_opt(tx, &write_opt).context("writing pending block to db")?;

        log::debug!("l2_pending_block_task: wrote pending block");
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
pub async fn sync(
    backend: &Arc<DeoxysBackend>,
    provider: SequencerGatewayProvider,
    config: L2SyncConfig,
    block_metrics: BlockMetrics,
    starting_block: u64,
    chain_id: Felt,
    telemetry: TelemetryHandle,
) -> anyhow::Result<()> {
    let (fetch_stream_sender, fetch_stream_receiver) = mpsc::channel(4);
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
    sync_timer: Arc<Mutex<Option<Instant>>>,
) -> anyhow::Result<()> {
    // Update Block sync time metrics
    let elapsed_time;
    {
        let mut timer_guard = sync_timer.lock().unwrap();
        if let Some(start_time) = *timer_guard {
            elapsed_time = start_time.elapsed().as_secs_f64();
            *timer_guard = Some(Instant::now());
        } else {
            // For the first block, there is no previous timer set
            elapsed_time = 0.0;
            *timer_guard = Some(Instant::now());
        }
    }

    let sync_time = block_metrics.l2_sync_time.get() + elapsed_time;
    block_metrics.l2_sync_time.set(sync_time);
    block_metrics.l2_latest_sync_time.set(elapsed_time);
    block_metrics.l2_avg_sync_time.set(block_metrics.l2_sync_time.get() / (block_number - starting_block) as f64);

    block_metrics.l2_block_number.set(block_header.block_number as f64);
    block_metrics.transaction_count.set(f64::from_u128(block_header.transaction_count).unwrap_or(f64::MIN));
    block_metrics.event_count.set(f64::from_u128(block_header.event_count).unwrap_or(f64::MIN));

    if let Some(l1_gas_price) = &block_header.l1_gas_price {
        block_metrics.l1_gas_price_wei.set(f64::from_u128(l1_gas_price.eth_l1_gas_price.into()).unwrap_or(f64::MIN));
        block_metrics.l1_gas_price_strk.set(f64::from_u128(l1_gas_price.strk_l1_gas_price.into()).unwrap_or(f64::MIN));
    }

    Ok(())
}

/// Verify and update the L2 state according to the latest state update
pub fn verify_l2(backend: &DeoxysBackend, block_number: u64, state_update: &StateUpdate) -> anyhow::Result<Felt> {
    let csd = build_commitment_state_diff(state_update);
    let state_root = csd_calculate_state_root(backend, csd, block_number);

    Ok(state_root)
}
