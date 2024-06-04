//! Contains the code required to sync data from the feeder efficiently.
use std::pin::pin;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{bail, Context};
use futures::{stream, StreamExt, TryStreamExt};
use mc_db::mapping_db::MappingDbError;
use mc_db::rocksdb::WriteBatchWithTransaction;
use mc_db::storage_handler::primitives::contract_class::{ClassUpdateWrapper, ContractClassData};
use mc_db::storage_handler::DeoxysStorageError;
use mc_db::storage_updates::{store_class_update, store_key_update, store_state_update};
use mc_db::DeoxysBackend;
use mp_block::{BlockId, BlockTag, DeoxysBlock};
use mp_felt::trim_hash;
use serde::Deserialize;
use starknet_api::hash::{StarkFelt, StarkHash};
use starknet_core::types::StateUpdate;
use starknet_providers::sequencer::models::StateUpdateWithBlock;
use starknet_providers::{ProviderError, SequencerGatewayProvider};
use tokio::sync::{mpsc, oneshot};
use tokio::time::Duration;

use crate::commitments::lib::{build_commitment_state_diff, csd_calculate_state_root};
use crate::convert::convert_block;
use crate::fetch::fetchers::L2BlockAndUpdates;
use crate::fetch::l2_fetch_task;
use crate::metrics::block_metrics::BlockMetrics;
use crate::stopwatch_end;
use crate::utils::PerfStopwatch;

/// Prefer this compared to [`tokio::spawn_blocking`], as spawn_blocking creates new OS threads and
/// we don't really need that
async fn spawn_compute<F, R>(func: F) -> R
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let (tx, rx) = tokio::sync::oneshot::channel();

    rayon::spawn(move || {
        let _result = tx.send(func());
    });

    rx.await.expect("tokio channel closed")
}

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
#[derive(Debug, Clone, Deserialize)]
pub struct L2StateUpdate {
    pub block_number: u64,
    pub global_root: StarkHash,
    pub block_hash: StarkHash,
}

fn store_new_block(block: &DeoxysBlock) -> Result<(), DeoxysStorageError> {
    let sw = PerfStopwatch::new();
    let mut tx = WriteBatchWithTransaction::default();

    let mapping = DeoxysBackend::mapping();
    mapping.write_new_block(&mut tx, block)?;

    let db_access = DeoxysBackend::expose_db();
    db_access.write(tx).map_err(MappingDbError::from)?;

    stopwatch_end!(sw, "end store_new_block {}: {:?}", block.block_n());
    Ok(())
}

async fn l2_verify_and_apply_task(
    mut updates_receiver: mpsc::Receiver<L2ConvertedBlockAndUpdates>,
    verify: bool,
    backup_every_n_blocks: Option<u64>,
    block_metrics: Option<BlockMetrics>,
    sync_timer: Arc<Mutex<Option<Instant>>>,
) -> anyhow::Result<()> {
    while let Some(L2ConvertedBlockAndUpdates { converted_block, state_update, class_update }) =
        pin!(updates_receiver.recv()).await
    {
        let block_n = converted_block.block_n();
        let block_hash = *converted_block.block_hash();
        let global_state_root = converted_block.header().global_state_root;

        let state_update = if verify {
            let state_update = Arc::new(state_update);
            let state_update_1 = Arc::clone(&state_update);

            let state_root = spawn_compute(move || {
                let sw = PerfStopwatch::new();
                let state_root = verify_l2(block_n, &state_update)?;
                stopwatch_end!(sw, "verify_l2: {:?}");

                anyhow::Ok(state_root)
            })
            .await?;

            if global_state_root != state_root {
                // TODO(fault tolerance): we should have a single rocksdb transaction for the whole l2 update.
                // let prev_block = block_n.checked_sub(1).expect("no block to revert to");

                // storage_handler::contract_trie_mut().revert_to(prev_block);
                // storage_handler::contract_storage_trie_mut().revert_to(prev_block);
                // storage_handler::contract_class_trie_mut().revert_to(prev_block);
                // TODO(charpa): make other stuff revertible, maybe history?

                bail!("Verified state: {} doesn't match fetched state: {}", state_root, global_state_root);
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
            || store_new_block(&converted_block),
            || {
                rayon::join(
                    || store_state_update(block_n, state_update),
                    || {
                        rayon::join(
                            || store_class_update(block_n, ClassUpdateWrapper(class_update)),
                            || store_key_update(block_n, &storage_diffs),
                        )
                    },
                )
            },
        );
        r1.and(r2).and(r3).and(r4).context("storing new block")?;

        update_sync_metrics(
            // &mut command_sink,
            block_n,
            block_metrics.as_ref(),
            sync_timer.clone(),
        )
        .await?;

        DeoxysBackend::meta().set_current_sync_block(block_n).context("setting current sync block")?;
        log::info!(
            "✨ Imported #{} ({}) and updated state root ({})",
            block_n,
            trim_hash(&block_hash.into()),
            trim_hash(&global_state_root.into())
        );
        // compact DB every 1k blocks
        if block_n % 1000 == 0 {
            DeoxysBackend::compact();
        }

        if backup_every_n_blocks.is_some_and(|backup_every_n_blocks| block_n % backup_every_n_blocks as u64 == 0) {
            log::info!("⏳ Backing up database at block {block_n}...");
            let sw = PerfStopwatch::new();
            DeoxysBackend::backup().await.context("backing up database")?;
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
    chain_id: StarkFelt,
) -> anyhow::Result<()> {
    // Items of this stream are futures that resolve to blocks, which becomes a regular stream of blocks
    // using futures buffered.
    let conversion_stream = stream::unfold((updates_receiver, chain_id), |(mut updates_recv, chain_id)| async move {
        match updates_recv.recv().await {
            Some(L2BlockAndUpdates { block, state_update, class_update, .. }) => Some((
                spawn_compute(move || {
                    let sw = PerfStopwatch::new();
                    let converted_block = convert_block(block, chain_id).context("converting block")?;
                    stopwatch_end!(sw, "convert_block: {:?}");
                    Ok(L2ConvertedBlockAndUpdates { converted_block, state_update, class_update })
                }),
                (updates_recv, chain_id),
            )),
            None => None,
        }
    });

    conversion_stream
        .buffered(10)
        .try_for_each(|block| async { output.send(block).await.context("downstream task is not running").map(|_| {}) })
        .await
}

async fn l2_pending_block_task(
    sync_finished_cb: oneshot::Receiver<()>,
    provider: Arc<SequencerGatewayProvider>,
    chain_id: StarkFelt,
) -> anyhow::Result<()> {
    // clear pending status
    {
        let mut tx = WriteBatchWithTransaction::default();
        DeoxysBackend::mapping().write_no_pending(&mut tx).context("clearing pending status")?;
        DeoxysBackend::expose_db().write(tx).context("writing pending block to db")?;
        log::debug!("l2_pending_block_task: startup: wrote no pending");
    }

    // we start the pending block task only once the node has been fully sync
    sync_finished_cb.await.context("sync is not running")?;

    let mut interval = tokio::time::interval(Duration::from_secs(2)); // TODO(cli): make interval configurable
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;

        let storage = DeoxysBackend::mapping();

        let StateUpdateWithBlock { state_update, block } = provider
            .get_state_update_with_block(starknet_providers::sequencer::models::BlockId::Pending)
            .await
            .context("getting pending block from sequencer")?;

        let block_n_best = storage.get_block_n(&BlockId::Tag(BlockTag::Latest))?.unwrap_or(0);
        let Some(block_n_pending_min_1) = block.block_number.unwrap().checked_sub(1) else {
            bail!("Pending block is genesis")
        };

        let mut tx = WriteBatchWithTransaction::default();
        if block_n_best == block_n_pending_min_1 {
            let block = spawn_compute(move || crate::convert::convert_block(block, chain_id))
                .await
                .context("converting pending block")?;
            let storage_update = crate::convert::state_update(state_update);
            storage
                .write_pending(&mut tx, &block, &storage_update)
                .context("writing pending to rocksdb transaction")?;
        } else {
            storage.write_no_pending(&mut tx).context("writing no pending to rocksdb transaction")?;
        }

        DeoxysBackend::expose_db().write(tx).context("writing pending block to db")?;

        log::debug!("l2_pending_block_task: wrote pending block");
    }
}

pub struct L2SyncConfig {
    pub first_block: u64,
    pub n_blocks_to_sync: Option<u64>,
    pub verify: bool,
    pub sync_polling_interval: Option<Duration>,
    pub backup_every_n_blocks: Option<u64>,
}

/// Spawns workers to fetch blocks and state updates from the feeder.
/// `n_blocks` is optionally the total number of blocks to sync, for debugging/benchmark purposes.
pub async fn sync(
    // block_sender: Sender<DeoxysBlock>,
    // command_sink: CommandSink,
    provider: SequencerGatewayProvider,
    config: L2SyncConfig,
    block_metrics: Option<BlockMetrics>,
    chain_id: StarkFelt,
) -> anyhow::Result<()> {
    let (fetch_stream_sender, fetch_stream_receiver) = mpsc::channel(30);
    let (block_conv_sender, block_conv_receiver) = mpsc::channel(30);
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

    let mut fetch_task = tokio::spawn(l2_fetch_task(
        config.first_block,
        config.n_blocks_to_sync,
        fetch_stream_sender,
        Arc::clone(&provider),
        config.sync_polling_interval,
        once_caught_up_cb_sender,
    ));
    let mut block_conversion_task =
        tokio::spawn(l2_block_conversion_task(fetch_stream_receiver, block_conv_sender, chain_id));
    let mut verify_and_apply_task = tokio::spawn(l2_verify_and_apply_task(
        block_conv_receiver,
        config.verify,
        config.backup_every_n_blocks,
        block_metrics,
        Arc::clone(&sync_timer),
    ));
    let mut pending_block_task = tokio::spawn(l2_pending_block_task(once_caught_up_cb_receiver, provider, chain_id));

    tokio::select!(
        // update pending block task
        res = &mut fetch_task => res.context("task was canceled")?,
        res = &mut block_conversion_task => res.context("task was canceled")?,
        res = &mut verify_and_apply_task => res.context("task was canceled")?,
        res = &mut pending_block_task => res.context("task was canceled")?,
        _ = tokio::signal::ctrl_c() => Ok(()), // graceful shutdown
    )?;

    // one of the task exited, which means it has dropped its channel and downstream tasks should be
    // able to detect that and gracefully finish their business this ensures no task outlive their
    // parent
    let (a, b, c, d) = tokio::join!(fetch_task, block_conversion_task, verify_and_apply_task, pending_block_task);
    a.context("task was canceled")?
        .and(b.context("task was canceled")?)
        .and(c.context("task was canceled")?)
        .and(d.context("task was canceled")?)?;
    Ok(())
}

async fn update_sync_metrics(
    block_number: u64,
    block_metrics: Option<&BlockMetrics>,
    sync_timer: Arc<Mutex<Option<Instant>>>,
) -> anyhow::Result<()> {
    // Update Block sync time metrics
    if let Some(block_metrics) = block_metrics {
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
        block_metrics.l2_avg_sync_time.set(block_metrics.l2_sync_time.get() / block_number as f64);
    }

    Ok(())
}

/// Verify and update the L2 state according to the latest state update
pub fn verify_l2(block_number: u64, state_update: &StateUpdate) -> anyhow::Result<StarkFelt> {
    let csd = build_commitment_state_diff(state_update);
    let state_root = csd_calculate_state_root(csd, block_number);

    Ok(state_root.into())
}
