//! Contains the code required to fetch data from the feeder efficiently.

use std::sync::Arc;
use std::time::Duration;
use reqwest::Url;
use sp_core::H256;
use blockifier::state::cached_state::CommitmentStateDiff;
use starknet_api::block::{BlockNumber, BlockHash};
use starknet_api::hash::StarkHash;
use starknet_gateway::sequencer::models::{BlockId, state_update};
use starknet_gateway::SequencerGatewayProvider;
use tokio::sync::mpsc::Sender;
use tokio::sync::Mutex;

use crate::CommandSink;

pub struct StarknetStateUpdate(starknet_gateway::sequencer::models::StateUpdate);

/// The configuration of the worker responsible for fetching new blocks and state updates from the feeder.
pub struct FetchConfig {
    /// The URL of the sequencer gateway.
    pub gateway: Url,
    /// The URL of the feeder gateway.
    pub feeder_gateway: Url,
    /// The ID of the chain served by the sequencer gateway.
    pub chain_id: starknet_ff::FieldElement,
    /// The number of tasks spawned to fetch blocks and state updates.
    pub workers: u32,
    /// Sender for dispatching fetched blocks.
    pub block_sender: Sender<mp_block::Block>,
    /// Sender for dispatching fetched state updates.
    pub state_update_sender: Sender<StarknetStateUpdate>,
}

/// Used to determine which Ids are required to be fetched.
struct IdServer {
    /// When `failed_ids` is empty, the next ID to fetch.
    next_id: u64,
    /// A list of IDs that have failed.
    failed_ids: Vec<u64>,
}

impl IdServer {
    /// Creates a new ID server.
    pub fn new(start_id: u64) -> Self {
        Self { next_id: start_id, failed_ids: Vec::new() }
    }

    /// Acquires an ID to fetch.
    pub fn acquire(&mut self) -> u64 {
        match self.failed_ids.pop() {
            Some(id) => id,
            None => {
                let id = self.next_id;
                self.next_id += 1;
                id
            }
        }
    }

    /// Releases an ID, scheduling it for a retry.
    pub fn release(&mut self, id: u64) {
        self.failed_ids.push(id);
    }
}

/// The state required to syncronize worker threads.
struct SyncState {
    /// The hash of the last sealed block.
    last_hash: Option<H256>,
    /// The block number of the next block to be sealed.
    next_number: u64,
}

impl SyncState {
    /// Creates a new sync state.
    #[inline]
    pub fn new(start_at: u64) -> Self {
        Self { last_hash: None, next_number: start_at }
    }
}

/// The state that is shared between fetch workers.
struct WorkerSharedState {
    /// The ID server.
    ids: Mutex<IdServer>,
    /// The client used to perform requests.
    client: SequencerGatewayProvider,
    /// The block sender.
    block_sender: Sender<mp_block::Block>,
    /// The state sender.
    state_update_sender: Sender<StarknetStateUpdate>,
    /// The state of the last block.
    sync_state: Mutex<SyncState>,
}

impl WorkerSharedState {
    async fn acquire_id(&self) -> u64 {
        let mut ids = self.ids.lock().await;
        ids.acquire()
    }

    async fn release_id(&self, id: u64) {
        let mut ids = self.ids.lock().await;
        ids.release(id);
    }
}

/// Spawns workers to fetch blocks and state updates from the feeder.
pub async fn sync(
    command_sink: CommandSink,
    config: FetchConfig,
    start_at: u64,
) {
    let shared_state = Arc::new(WorkerSharedState {
        client: SequencerGatewayProvider::new(config.gateway, config.feeder_gateway, config.chain_id),
        block_sender: config.block_sender,
        state_update_sender: config.state_update_sender,
        ids: Mutex::new(IdServer::new(start_at)),
        sync_state: Mutex::new(SyncState::new(start_at)),
    });

    for _ in 0..config.workers {
        let state = shared_state.clone();
        let command_sink_clone = command_sink.clone();
        tokio::spawn(async move {
            start_worker(state, command_sink_clone, WorkerType::Block).await;
        });
        let state = shared_state.clone();
        let command_sink_clone = command_sink.clone();
        tokio::spawn(async move {
            start_worker(state, command_sink_clone, WorkerType::StateUpdate).await;
        });
    }
}

enum WorkerType {
    Block,
    StateUpdate,
}

/// Starts a generic worker task.
async fn start_worker(
    state: Arc<WorkerSharedState>,
    mut command_sink: CommandSink,
    worker_type: WorkerType,
) {
    loop {
        let id = state.acquire_id().await;
        let result = match worker_type {
            WorkerType::Block => get_and_dispatch_block(&state, id, &mut command_sink).await,
            WorkerType::StateUpdate => get_and_dispatch_state_update(&state, id).await,
        };

        if let Err(err) = result {
            state.release_id(id).await;
            eprintln!("Error sending {}: {:?}", match worker_type {
                WorkerType::Block => "block",
                WorkerType::StateUpdate => "state update",
            }, err);
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    }
}

/// Gets a block from the network and sends it to the other half of the channel.
async fn get_and_dispatch_block(
    state: &WorkerSharedState,
    block_id: u64,
    command_sink: &mut CommandSink,
) -> Result<(), String> {
    let block =
        state.client.get_block(BlockId::Number(block_id)).await.map_err(|e| format!("failed to get block: {e}"))?;
    let block = super::convert::block(&block);

    let mut lock;
    loop {
        lock = state.sync_state.lock().await;
        if lock.next_number == block_id {
            break;
        }
        drop(lock);
        tokio::task::yield_now().await;
    }

    state.block_sender.send(block).await.map_err(|e| format!("failed to dispatch block: {e}"))?;
    let hash = create_block(command_sink, lock.last_hash).await?;

    lock.last_hash = Some(hash);
    lock.next_number += 1;

    Ok(())
}

/// Gets a state update from the network and sends it to the other half of the channel.
async fn get_and_dispatch_state_update(
    state: &WorkerSharedState,
    state_id: u64,
) -> Result<(), String> {
    // Replace `get_state_update` with the actual function call to fetch the state update.
    let state_update_result = state.client.get_state_update(BlockId::Number(state_id)).await;

    // Check if the state update was fetched successfully.
    let state_update = match state_update_result {
        Ok(update) => update,
        Err(e) => {
            let error_message = format!("failed to get state update for id {}: {}", state_id, e);
            eprintln!("{}", &error_message);
            return Err(error_message);
        }
    };

    // Lock the sync state to check if it's the correct turn to dispatch this state update.
    let mut lock = state.sync_state.lock().await;
    if lock.next_number != state_id {
        // If not, release the lock and yield control to the scheduler.
        drop(lock);
        tokio::task::yield_now().await;
        return Err(format!("State update id {} came out of order", state_id));
    }

    // Send the state update.
    state.state_update_sender.send(StarknetStateUpdate(state_update)).await.map_err(|e| format!("failed to dispatch state update: {e}"))?;

    // Increment the next number to process the next state update.
    lock.next_number += 1;

    Ok(())
}

/// Notifies the consensus engine that a new block should be created.
async fn create_block(command_sink: &mut CommandSink, _parent_hash: Option<H256>) -> Result<H256, String> {
    let (sender, receiver) = futures::channel::oneshot::channel();

    command_sink
        .try_send(sc_consensus_manual_seal::rpc::EngineCommand::SealNewBlock {
            create_empty: true,
            finalize: true,
            parent_hash: None,
            sender: Some(sender),
        })
        .unwrap();

    let create_block_info = receiver
        .await
        .map_err(|err| format!("failed to seal block: {err}"))?
        .map_err(|err| format!("failed to seal block: {err}"))?;
    Ok(create_block_info.hash)
}