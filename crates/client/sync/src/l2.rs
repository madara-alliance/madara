//! Contains the code required to sync data from the feeder efficiently.
use std::pin::pin;
use std::str::FromStr;
use std::sync::{Arc, RwLock};

use lazy_static::lazy_static;
use mc_db::storage_handler::primitives::contract_class::ClassUpdateWrapper;
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
use starknet_providers::sequencer::models::BlockId;
use starknet_providers::{ProviderError, SequencerGatewayProvider};
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;
use tokio::time::Duration;

use crate::commitments::lib::{build_commitment_state_diff, update_state_root};
use crate::fetch::fetchers::L2BlockAndUpdates;
use crate::fetch::l2_fetch_task;
use crate::l1::ETHEREUM_STATE_UPDATE;
use crate::CommandSink;

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
#[derive(Error, Debug)]
pub enum L2SyncError {
    #[error("provider error")]
    Provider(#[from] ProviderError),
    #[error("fetch retry limit exceeded")]
    FetchRetryLimit,
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
    mut fetch_stream_receiver: mpsc::Receiver<L2BlockAndUpdates>,
    block_sender: Sender<DeoxysBlock>,
    mut command_sink: CommandSink,
    verify: bool,
) -> Result<(), L2SyncError> {
    let block_sender = Arc::new(block_sender);

    let mut last_block_hash = None;

    while let Some(L2BlockAndUpdates { block_n, block, state_update, class_update }) =
        pin!(fetch_stream_receiver.recv()).await
    {
        let (state_update, block_conv) = {
            let state_update = Arc::new(state_update);
            let state_update_1 = Arc::clone(&state_update);

            let block_conv = spawn_compute(move || {
                let convert_block = |block| {
                    let start = std::time::Instant::now();
                    let block_conv = crate::convert::convert_block_sync(block);
                    log::debug!("convert::convert_block_sync {}: {:?}", block_n, std::time::Instant::now() - start);
                    block_conv
                };
                let ver_l2 = || {
                    let start = std::time::Instant::now();
                    let state_root = verify_l2(block_n, &state_update);
                    log::debug!("verify_l2: {:?}", std::time::Instant::now() - start);
                    state_root
                };

                if verify {
                    let (state_root, block_conv) = rayon::join(ver_l2, || convert_block(block));
                    if (block_conv.header().global_state_root) != state_root {
                        log::info!(
                            "❗ Verified state: {} doesn't match fetched state: {}",
                            state_root,
                            block_conv.header().global_state_root
                        );
                    }
                    block_conv
                } else {
                    convert_block(block)
                }
            })
            .await;

            // UNWRAP: we need a 'static future as we are spawning tokio tasks further down the line
            //         this is a hack to achieve that, we put the update in an arc and then unwrap it at the end
            //         this will not panic as the Arc should not be aliased.
            let state_update = Arc::try_unwrap(state_update_1).unwrap();
            (state_update, block_conv)
        };

        let block_sender = Arc::clone(&block_sender);
        let storage_diffs = state_update.state_diff.storage_diffs.clone();
        tokio::join!(
            async move {
                block_sender.send(block_conv).await.expect("block reciever channel is closed");
            },
            async {
                let start = std::time::Instant::now();
                if store_state_update(block_n, state_update).await.is_err() {
                    log::info!("❗ Failed to store state update for block {block_n}");
                };
                log::debug!("end store_state {}: {:?}", block_n, std::time::Instant::now() - start);
            },
            async {
                let start = std::time::Instant::now();
                if store_class_update(block_n, ClassUpdateWrapper(class_update)).await.is_err() {
                    log::info!("❗ Failed to store class update for block {block_n}");
                };
                log::debug!("end store_class {}: {:?}", block_n, std::time::Instant::now() - start);
            },
            async {
                let start = std::time::Instant::now();
                if store_key_update(block_n, &storage_diffs).await.is_err() {
                    log::info!("❗ Failed to store key update for block {block_n}");
                };
                log::debug!("end store_key {}: {:?}", block_n, std::time::Instant::now() - start);
            },
            async {
                let start = std::time::Instant::now();
                create_block(&mut command_sink, &mut last_block_hash).await.expect("creating block");
                log::debug!("end create_block {}: {:?}", block_n, std::time::Instant::now() - start);
            }
        );

        // compact DB every 1k blocks
        if block_n % 1000 == 0 {
            DeoxysBackend::compact();
        }
    }

    Ok(())
}

/// Spawns workers to fetch blocks and state updates from the feeder.
/// `n_blocks` is optionally the total number of blocks to sync, for debugging/benchmark purposes.
pub async fn sync<C>(
    block_sender: Sender<DeoxysBlock>,
    command_sink: CommandSink,
    provider: SequencerGatewayProvider,
    first_block: u64,
    verify: bool,
    client: Arc<C>,
    pending_polling_interval: Duration,
) -> Result<(), L2SyncError>
where
    C: HeaderBackend<DBlockT> + 'static,
{
    let (fetch_stream_sender, fetch_stream_receiver) = mpsc::channel(10);
    let provider = Arc::new(provider);

    // [Fetch task] ==new blocks and updates=> [Verification and apply task]
    // Fetch task does parallel fetching, verification is sequential and does all the compute and db
    // updates

    // TODO: make it cancel-safe, tasks outlive their parent here when error occurs here
    // we are using separate tasks so that fetches don't get clogged up if by any chance the verify task
    // starves the tokio worker
    tokio::select!(
        // update highest block hash and number, update pending block and state update
        // TODO: remove
        _ = async {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if let Err(e) = update_starknet_data(&provider, client.as_ref()).await {
                    log::error!("Failed to update highest block hash and number: {}", e);
                }
            }
        } => Ok(()),
        // fetch blocks and updates in parallel
        res = tokio::spawn(l2_fetch_task(first_block, fetch_stream_sender, Arc::clone(&provider), pending_polling_interval)) => res.expect("join error"),
        // apply blocks and updates sequentially
        res = tokio::spawn(l2_verify_and_apply_task(fetch_stream_receiver, block_sender, command_sink, verify)) => res.expect("join error"),
    )?;

    Ok(())
}

/// Notifies the consensus engine that a new block should be created.
async fn create_block(cmds: &mut CommandSink, parent_hash: &mut Option<H256>) -> Result<(), String> {
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
pub fn verify_l2(block_number: u64, state_update: &StateUpdate) -> StarkFelt {
    let csd = build_commitment_state_diff(state_update);
    let state_root = update_state_root(csd, block_number);
    let block_hash = state_update.block_hash;

    update_l2(L2StateUpdate {
        block_number,
        global_root: state_root.into(),
        block_hash: Felt252Wrapper::from(block_hash).into(),
    });

    state_root.into()
}

async fn update_starknet_data<C>(provider: &SequencerGatewayProvider, client: &C) -> Result<(), String>
where
    C: HeaderBackend<DBlockT>,
{
    let block = provider.get_block(BlockId::Pending).await.map_err(|e| format!("Failed to get pending block: {e}"))?;

    let hash_best = client.info().best_hash;
    let hash_current = block.parent_block_hash;
    let number = provider
        .get_block_id_by_hash(hash_current)
        .await
        .map_err(|e| format!("Failed to get block id by hash: {e}"))?;
    let tmp = DHashT::from_str(&hash_current.to_string()).unwrap_or(Default::default());

    if hash_best == tmp {
        let state_update = provider
            .get_state_update(BlockId::Pending)
            .await
            .map_err(|e| format!("Failed to get pending state update: {e}"))?;

        *STARKNET_PENDING_BLOCK.write().expect("Failed to acquire write lock on STARKNET_PENDING_BLOCK") =
            Some(crate::convert::block(block).await);

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
