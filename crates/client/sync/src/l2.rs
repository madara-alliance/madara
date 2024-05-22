//! Contains the code required to sync data from the feeder efficiently.
use std::pin::pin;
use std::str::FromStr;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use anyhow::{bail, Context};
use futures::{stream, StreamExt, TryStreamExt};
use lazy_static::lazy_static;
use mc_db::storage_handler::primitives::contract_class::{ClassUpdateWrapper, ContractClassData};
use mc_db::storage_handler::DeoxysStorageError;
use mc_db::storage_updates::{store_class_update, store_key_update, store_state_update};
use mc_db::DeoxysBackend;
use mp_block::DeoxysBlock;
use mp_felt::Felt252Wrapper;
use mp_types::block::{DBlockT, DHashT};
use serde::Deserialize;
use sp_blockchain::HeaderBackend;
use sp_core::H256;
use starknet_api::hash::{StarkFelt, StarkHash};
use starknet_core::types::{PendingStateUpdate, StateUpdate};
use starknet_ff::FieldElement;
use starknet_providers::sequencer::models::{BlockId, StateUpdateWithBlock};
use starknet_providers::{ProviderError, SequencerGatewayProvider};
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;
use tokio::time::Duration;

use crate::commitments::lib::{build_commitment_state_diff, update_state_root};
use crate::convert::convert_block;
use crate::fetch::fetchers::L2BlockAndUpdates;
use crate::fetch::l2_fetch_task;
use crate::l1::ETHEREUM_STATE_UPDATE;
use crate::metrics::block_metrics::BlockMetrics;
use crate::utils::PerfStopwatch;
use crate::{stopwatch_end, CommandSink};

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
}

/// Contains the latest Starknet verified state on L2
#[derive(Debug, Clone, Deserialize)]
pub struct L2StateUpdate {
    pub block_number: u64,
    pub global_root: StarkHash,
    pub block_hash: StarkHash,
}

/// The current syncing status:
///
/// - SyncVerifiedState: the node is syncing AcceptedOnL1 blocks
/// - SyncUnverifiedState: the node is syncing AcceptedOnL2 blocks
/// - SyncPendingState: the node is fully synced and now syncing Pending blocks
///
/// This is used to determine the current state of the syncing process
pub enum SyncStatus {
    SyncVerifiedState,
    SyncUnverifiedState,
    SyncPendingState,
}

lazy_static! {
    /// Shared current syncing status, either verified, unverified or pending
    pub static ref SYNC_STATUS: RwLock<SyncStatus> = RwLock::new(SyncStatus::SyncVerifiedState);
}

lazy_static! {
    /// Shared latest L2 state update verified on L2
    pub static ref STARKNET_STATE_UPDATE: RwLock<L2StateUpdate> = RwLock::new(L2StateUpdate {
        block_number: u64::default(),
        global_root: StarkHash::default(),
        block_hash: StarkHash::default(),
    });
}

lazy_static! {
    /// Shared latest block number and hash of chain, using a RwLock to allow for concurrent reads and exclusive writes
    pub static ref STARKNET_HIGHEST_BLOCK_HASH_AND_NUMBER: RwLock<(FieldElement, u64)> = RwLock::new((FieldElement::default(), 0));
}

lazy_static! {
    /// Shared pending block data, using a RwLock to allow for concurrent reads and exclusive writes
    static ref STARKNET_PENDING_BLOCK: RwLock<Option<DeoxysBlock>> = RwLock::new(None);
}

lazy_static! {
    /// Shared pending state update, using RwLock to allow for concurrent reads and exclusive writes
    static ref STARKNET_PENDING_STATE_UPDATE: RwLock<Option<PendingStateUpdate>> = RwLock::new(None);
}

pub fn get_highest_block_hash_and_number() -> (FieldElement, u64) {
    *STARKNET_HIGHEST_BLOCK_HASH_AND_NUMBER
        .read()
        .expect("Failed to acquire read lock on STARKNET_HIGHEST_BLOCK_HASH_AND_NUMBER")
}

pub fn get_pending_block() -> Option<DeoxysBlock> {
    STARKNET_PENDING_BLOCK.read().expect("Failed to acquire read lock on STARKNET_PENDING_BLOCK").clone()
}

pub fn get_pending_state_update() -> Option<PendingStateUpdate> {
    STARKNET_PENDING_STATE_UPDATE.read().expect("Failed to acquire read lock on STARKNET_PENDING_BLOCK").clone()
}

/// The configuration of the senders responsible for sending blocks and state
/// updates from the feeder.
pub struct SenderConfig {
    /// Sender for dispatching fetched blocks.
    pub block_sender: Sender<DeoxysBlock>,
    /// The command sink used to notify the consensus engine that a new block
    /// should be created.
    pub command_sink: CommandSink,
}

async fn l2_verify_and_apply_task(
    mut updates_receiver: mpsc::Receiver<L2ConvertedBlockAndUpdates>,
    block_sender: Sender<DeoxysBlock>,
    mut command_sink: CommandSink,
    verify: bool,
    backup_every_n_blocks: Option<usize>,
    block_metrics: Option<BlockMetrics>,
    sync_timer: Arc<Mutex<Option<Instant>>>,
) -> anyhow::Result<()> {
    let block_sender = Arc::new(block_sender);

    let mut last_block_hash = None;

    while let Some(L2ConvertedBlockAndUpdates { block_n, block, state_update, class_update }) =
        pin!(updates_receiver.recv()).await
    {
        let state_update = if verify {
            let state_update = Arc::new(state_update);
            let state_update_1 = Arc::clone(&state_update);
            let global_state_root = block.header().global_state_root;

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

        let block_sender = Arc::clone(&block_sender);
        let storage_diffs = state_update.state_diff.storage_diffs.clone();
        tokio::join!(
            async move {
                block_sender.send(block).await.expect("block reciever channel is closed");
            },
            async {
                let sw = PerfStopwatch::new();
                if store_state_update(block_n, state_update).await.is_err() {
                    log::error!("❗ Failed to store state update for block {block_n}");
                };
                stopwatch_end!(sw, "end store_state {}: {:?}", block_n);
            },
            async {
                let sw = PerfStopwatch::new();
                if store_class_update(block_n, ClassUpdateWrapper(class_update)).await.is_err() {
                    log::error!("❗ Failed to store class update for block {block_n}");
                };
                stopwatch_end!(sw, "end store_class {}: {:?}", block_n);
            },
            async {
                let sw = PerfStopwatch::new();
                if store_key_update(block_n, &storage_diffs).await.is_err() {
                    log::error!("❗ Failed to store key update for block {block_n}");
                };
                stopwatch_end!(sw, "end store_key {}: {:?}", block_n);
            },
            async {
                let sw = PerfStopwatch::new();
                create_block(
                    &mut command_sink,
                    &mut last_block_hash,
                    block_n,
                    block_metrics.clone(),
                    sync_timer.clone(),
                )
                .await
                .expect("creating block");
                stopwatch_end!(sw, "end create_block {}: {:?}", block_n);
            }
        );

        DeoxysBackend::meta().set_current_sync_block(block_n).context("setting current sync block")?;

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
    pub block_n: u64,
    pub block: DeoxysBlock,
    pub state_update: StateUpdate,
    pub class_update: Vec<ContractClassData>,
}

async fn l2_block_conversion_task(
    updates_receiver: mpsc::Receiver<L2BlockAndUpdates>,
    output: mpsc::Sender<L2ConvertedBlockAndUpdates>,
) -> anyhow::Result<()> {
    // Items of this stream are futures that resolve to blocks, which becomes a regular stream of blocks
    // using futures buffered.
    let conversion_stream = stream::unfold(updates_receiver, |mut updates_recv| async {
        updates_recv.recv().await.map(|L2BlockAndUpdates { block_n, block, state_update, class_update }| {
            (
                spawn_compute(move || {
                    let sw = PerfStopwatch::new();
                    let block = convert_block(block)?;
                    stopwatch_end!(sw, "convert_block: {:?}");
                    Ok(L2ConvertedBlockAndUpdates { block_n, block, state_update, class_update })
                }),
                updates_recv,
            )
        })
    });

    conversion_stream
        .buffered(10)
        .try_for_each(|block| async {
            output.send(block).await.expect("downstream task is not running");
            Ok(())
        })
        .await
}

pub struct L2SyncConfig {
    pub first_block: u64,
    pub n_blocks_to_sync: Option<u64>,
    pub verify: bool,
    pub sync_polling_interval: Option<Duration>,
    pub backup_every_n_blocks: Option<usize>,
}

/// Spawns workers to fetch blocks and state updates from the feeder.
/// `n_blocks` is optionally the total number of blocks to sync, for debugging/benchmark purposes.
pub async fn sync<C>(
    block_sender: Sender<DeoxysBlock>,
    command_sink: CommandSink,
    provider: SequencerGatewayProvider,
    client: Arc<C>,
    config: L2SyncConfig,
    block_metrics: Option<BlockMetrics>,
) -> anyhow::Result<()>
where
    C: HeaderBackend<DBlockT> + 'static,
{
    let (fetch_stream_sender, fetch_stream_receiver) = mpsc::channel(10);
    let (block_conv_sender, block_conv_receiver) = mpsc::channel(10);
    let provider = Arc::new(provider);
    let sync_timer = Arc::new(Mutex::new(None));

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
    ));
    let mut block_conversion_task = tokio::spawn(l2_block_conversion_task(fetch_stream_receiver, block_conv_sender));
    let mut verify_and_apply_task = tokio::spawn(l2_verify_and_apply_task(
        block_conv_receiver,
        block_sender,
        command_sink,
        config.verify,
        config.backup_every_n_blocks,
        block_metrics.clone(),
        Arc::clone(&sync_timer),
    ));

    tokio::select!(
        // update highest block hash and number, update pending block and state update
        // TODO: remove
        _ = async {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if let Err(e) = update_starknet_data(&provider, client.as_ref()).await {
                    log::error!("{:#}", e);
                }
            }
        } => Ok(()),
        res = &mut fetch_task => res.context("task was canceled")?,
        res = &mut block_conversion_task => res.context("task was canceled")?,
        res = &mut verify_and_apply_task => res.context("task was canceled")?,
        _ = tokio::signal::ctrl_c() => Ok(()), // graceful shutdown
    )?;

    // one of the task exited, which means it has dropped its channel and downstream tasks should be
    // able to detect that and gracefully finish their business this ensures no task outlive their
    // parent
    let (a, b, c) = tokio::join!(fetch_task, block_conversion_task, verify_and_apply_task);
    a.context("task was canceled")?.and(b.context("task was canceled")?).and(c.context("task was canceled")?)?;
    Ok(())
}

/// Notifies the consensus engine that a new block should be created.
async fn create_block(
    cmds: &mut CommandSink,
    parent_hash: &mut Option<H256>,
    block_number: u64,
    block_metrics: Option<BlockMetrics>,
    sync_timer: Arc<Mutex<Option<Instant>>>,
) -> Result<(), String> {
    let (sender, receiver) = futures::channel::oneshot::channel();

    cmds.try_send(sc_consensus_manual_seal::rpc::EngineCommand::SealNewBlock {
        create_empty: true,
        finalize: false,
        parent_hash: None,
        sender: Some(sender),
    })
    .unwrap();

    let create_block_info = receiver
        .await
        .map_err(|err| format!("failed to seal block: {err}"))?
        .map_err(|err| format!("failed to seal block: {err}"))?;

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

    *parent_hash = Some(create_block_info.hash);
    Ok(())
}

/// Update the L2 state with the latest data
pub fn update_l2(state_update: L2StateUpdate) {
    *STARKNET_STATE_UPDATE.write().expect("Failed to acquire write lock on STARKNET_STATE_UPDATE") =
        state_update.clone();

    let last_l1_state_update_block =
        ETHEREUM_STATE_UPDATE.read().expect("Failed to acquire read lock on ETHEREUM_STATE_UPDATE").block_number;
    if state_update.block_number >= last_l1_state_update_block {
        *SYNC_STATUS.write().expect("Failed to acquire write lock on SYNC_STATUS") = SyncStatus::SyncUnverifiedState;
    }
}

/// Verify and update the L2 state according to the latest state update
pub fn verify_l2(block_number: u64, state_update: &StateUpdate) -> anyhow::Result<StarkFelt> {
    let csd = build_commitment_state_diff(state_update);
    let state_root = update_state_root(csd, block_number);
    let block_hash = state_update.block_hash;

    update_l2(L2StateUpdate {
        block_number,
        global_root: state_root.into(),
        block_hash: Felt252Wrapper::from(block_hash).into(),
    });

    Ok(state_root.into())
}

async fn update_starknet_data<C>(provider: &SequencerGatewayProvider, client: &C) -> anyhow::Result<()>
where
    C: HeaderBackend<DBlockT>,
{
    let StateUpdateWithBlock { state_update, block } =
        provider.get_state_update_with_block(BlockId::Pending).await.context("Failed to get pending block")?;

    let hash_best = client.info().best_hash;
    let hash_current = block.parent_block_hash;
    let number = provider.get_block_id_by_hash(hash_current).await.context("Failed to get block id by hash")?;
    let tmp = DHashT::from_str(&hash_current.to_string()).unwrap_or(Default::default());

    if hash_best == tmp {
        *STARKNET_PENDING_BLOCK.write().expect("Failed to acquire write lock on STARKNET_PENDING_BLOCK") =
            Some(spawn_compute(|| crate::convert::convert_block(block)).await.unwrap());

        *STARKNET_PENDING_STATE_UPDATE.write().expect("Failed to aquire write lock on STARKNET_PENDING_STATE_UPDATE") =
            Some(crate::convert::state_update(state_update));
    }

    *STARKNET_HIGHEST_BLOCK_HASH_AND_NUMBER
        .write()
        .expect("Failed to acquire write lock on STARKNET_HIGHEST_BLOCK_HASH_AND_NUMBER") = (hash_current, number);

    log::debug!(
        "update_starknet_data: latest_block_number: {}, latest_block_hash: 0x{:x}, best_hash: {}",
        number,
        hash_current,
        hash_best
    );
    Ok(())
}
