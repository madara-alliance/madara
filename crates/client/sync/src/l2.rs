//! Contains the code required to sync data from the feeder efficiently.
use std::pin::pin;
use std::str::FromStr;
use std::sync::{Arc, Mutex, RwLock};

use bonsai_trie::id::BasicId;
use bonsai_trie::BonsaiStorage;
use futures::{stream, StreamExt};
use lazy_static::lazy_static;
use mc_db::bonsai_db::BonsaiDb;
use mc_storage::OverrideHandle;
use mp_block::state_update::StateUpdateWrapper;
use mp_contract::class::ClassUpdateWrapper;
use mp_felt::Felt252Wrapper;
use serde::Deserialize;
use sp_blockchain::HeaderBackend;
use sp_core::H256;
use sp_runtime::generic::{Block, Header};
use sp_runtime::traits::{BlakeTwo256, Block as BlockT};
use sp_runtime::OpaqueExtrinsic;
use starknet_api::hash::StarkHash;
use starknet_core::types::{PendingStateUpdate, StarknetError};
use starknet_ff::FieldElement;
use starknet_providers::sequencer::models::{BlockId, StateUpdate};
use starknet_providers::{ProviderError, SequencerGatewayProvider};
use starknet_types_core::hash::{Pedersen, Poseidon};
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;
use tokio::time::{Duration, Instant};

use crate::commitments::lib::{build_commitment_state_diff, update_state_root};
use crate::fetch::fetch::{fetch_block_and_updates, FetchConfig};
use crate::utility::block_hash_substrate;
use crate::CommandSink;

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

lazy_static! {
    /// Shared latest L2 state update verified on L2
    pub static ref STARKNET_STATE_UPDATE: Mutex<L2StateUpdate> = Mutex::new(L2StateUpdate {
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
    static ref STARKNET_PENDING_BLOCK: RwLock<Option<mp_block::Block>> = RwLock::new(None);
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

pub fn get_pending_block() -> Option<mp_block::Block> {
    STARKNET_PENDING_BLOCK.read().expect("Failed to acquire read lock on STARKNET_PENDING_BLOCK").clone()
}

pub fn get_pending_state_update() -> Option<PendingStateUpdate> {
    STARKNET_PENDING_STATE_UPDATE.read().expect("Failed to acquire read lock on STARKNET_PENDING_BLOCK").clone()
}

/// The configuration of the senders responsible for sending blocks and state
/// updates from the feeder.
pub struct SenderConfig {
    /// Sender for dispatching fetched blocks.
    pub block_sender: Sender<mp_block::Block>,
    /// Sender for dispatching fetched state updates.
    pub state_update_sender: Sender<StateUpdateWrapper>,
    /// Sender for dispatching fetched class hashes.
    pub class_sender: Sender<ClassUpdateWrapper>,
    /// The command sink used to notify the consensus engine that a new block
    /// should be created.
    pub command_sink: CommandSink,
    // Storage overrides for accessing stored classes
    pub overrides: Arc<OverrideHandle<Block<Header<u32, BlakeTwo256>, OpaqueExtrinsic>>>,
}

/// Spawns workers to fetch blocks and state updates from the feeder.
pub async fn sync<B, C>(
    mut sender_config: SenderConfig,
    fetch_config: FetchConfig,
    first_block: u64,
    bonsai_contract: &Arc<Mutex<BonsaiStorage<BasicId, BonsaiDb<B>, Pedersen>>>,
    bonsai_contract_storage: &Arc<Mutex<BonsaiStorage<BasicId, BonsaiDb<B>, Pedersen>>>,
    bonsai_class: &Arc<Mutex<BonsaiStorage<BasicId, BonsaiDb<B>, Poseidon>>>,
    client: Arc<C>,
) where
    B: BlockT,
    C: HeaderBackend<B>,
{
    let SenderConfig { block_sender, state_update_sender, class_sender, command_sink, overrides } = &mut sender_config;
    let provider = SequencerGatewayProvider::new(
        fetch_config.gateway.clone(),
        fetch_config.feeder_gateway.clone(),
        fetch_config.chain_id,
    );
    let mut last_block_hash = None;

    // TODO: move this somewhere else
    if first_block == 1 {
        let state_update =
            provider.get_state_update(BlockId::Number(0)).await.expect("getting state update for genesis block");
        verify_l2(0, &state_update, overrides, bonsai_contract, bonsai_contract_storage, bonsai_class, None)
            .expect("verifying genesis block");
    }

    let fetch_stream =
        (first_block..).map(|block_n| fetch_block_and_updates(block_n, &provider, overrides, client.as_ref()));
    // Have 10 fetches in parallel at once, using futures Buffered
    let mut fetch_stream = stream::iter(fetch_stream).buffered(10);
    let (fetch_stream_sender, mut fetch_stream_receiver) = mpsc::channel(10);

    let mut instant = Instant::now();

    tokio::select!(
        // fetch blocks and updates in parallel
        _ = async {
            // FIXME: make it cleaner by replacing this with tokio_util::sync::PollSender to make the channel a
            // Sink and have the fetch Stream pipe into it
            while let Some(val) = pin!(fetch_stream.next()).await {
                fetch_stream_sender.send(val).await.expect("receiver is closed");

                // tries to update the pending starknet block every 2s
                if instant.elapsed() >= Duration::from_secs(2) {
                    if let Err(e) = update_starknet_data(&provider, client.as_ref()).await {
                        log::error!("Failed to update highest block hash and number: {}", e);
                    }
                    instant = Instant::now();
                }
            }
        } => {},
        // apply blocks and updates sequentially
        _ = async {
            let mut block_n = first_block;
            while let Some(val) = pin!(fetch_stream_receiver.recv()).await {
                if matches!(val, Err(L2SyncError::Provider(ProviderError::StarknetError(StarknetError::BlockNotFound)))) {
                    break;
                }

                let (block, state_update, class_update) = val.expect("fetching block");

                let block_hash = block_hash_substrate(client.as_ref(), block_n - 1);

                let state_update = {
                    if fetch_config.verify {
                        let overrides = Arc::clone(overrides);
                        let bonsai_contract = Arc::clone(bonsai_contract);
                        let bonsai_contract_storage = Arc::clone(bonsai_contract_storage);
                        let bonsai_class = Arc::clone(bonsai_class);
                        let state_update = Arc::new(state_update);
                        let state_update_1 = Arc::clone(&state_update);

                        tokio::task::spawn_blocking(move || {
                            verify_l2(block_n, &state_update, &overrides, &bonsai_contract, &bonsai_contract_storage, &bonsai_class, block_hash)
                                .expect("verifying block");
                        })
                        .await
                        .expect("verification task panicked");

                        Arc::try_unwrap(state_update_1).expect("arc should not be aliased")
                    } else {
                        state_update
                    }
                };

                tokio::join!(
                    async {
                        let block_conv = crate::convert::block(block).await;
                        block_sender.send(block_conv).await.expect("block reciever channel is closed");
                    },
                    async {
                        // Now send state_update, which moves it. This will be received
                        // by QueryBlockConsensusDataProvider in deoxys/crates/node/src/service.rs
                        state_update_sender
                            .send(StateUpdateWrapper::from(state_update))
                            .await
                            .expect("state updater is not running");
                    },
                    async {
                        // do the same to class update
                        class_sender
                            .send(ClassUpdateWrapper(class_update))
                            .await
                            .expect("class updater is not running");
                    }
                );

                create_block(command_sink, &mut last_block_hash).await.expect("creating block");
                block_n += 1;
            }
        } => {},
    );

    log::debug!("L2 sync finished :)");
}

/// Notifies the consensus engine that a new block should be created.
async fn create_block(cmds: &mut CommandSink, parent_hash: &mut Option<H256>) -> Result<(), String> {
    let (sender, receiver) = futures::channel::oneshot::channel();

    cmds.try_send(sc_consensus_manual_seal::rpc::EngineCommand::SealNewBlock {
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

    *parent_hash = Some(create_block_info.hash);
    Ok(())
}

/// Update the L2 state with the latest data
pub fn update_l2(state_update: L2StateUpdate) {
    let mut last_state_update = STARKNET_STATE_UPDATE.lock().expect("Failed to acquire lock on STARKNET_STATE_UPDATE");
    *last_state_update = state_update.clone();
}

/// Verify and update the L2 state according to the latest state update
pub fn verify_l2<B: BlockT>(
    block_number: u64,
    state_update: &StateUpdate,
    overrides: &Arc<OverrideHandle<Block<Header<u32, BlakeTwo256>, OpaqueExtrinsic>>>,
    bonsai_contract: &Arc<Mutex<BonsaiStorage<BasicId, BonsaiDb<B>, Pedersen>>>,
    bonsai_contract_storage: &Arc<Mutex<BonsaiStorage<BasicId, BonsaiDb<B>, Pedersen>>>,
    bonsai_class: &Arc<Mutex<BonsaiStorage<BasicId, BonsaiDb<B>, Poseidon>>>,
    substrate_block_hash: Option<H256>,
) -> Result<(), L2SyncError> {
    let state_update_wrapper = StateUpdateWrapper::from(state_update);

    let csd = build_commitment_state_diff(state_update_wrapper.clone());
    let state_root = update_state_root(
        csd,
        Arc::clone(overrides),
        Arc::clone(bonsai_contract),
        Arc::clone(bonsai_contract_storage),
        Arc::clone(bonsai_class),
        block_number,
        substrate_block_hash,
    );
    log::debug!("state_root: {state_root:?}");
    let block_hash = state_update.block_hash.expect("Block hash not found in state update");
    log::debug!("update_state_root {} -- block_hash: {block_hash:?}, state_root: {state_root:?}", block_number);

    update_l2(L2StateUpdate {
        block_number,
        global_root: state_root.into(),
        block_hash: Felt252Wrapper::from(block_hash).into(),
    });

    Ok(())
}

async fn update_starknet_data<B, C>(provider: &SequencerGatewayProvider, client: &C) -> Result<(), String>
where
    B: BlockT,
    C: HeaderBackend<B>,
{
    let block = provider.get_block(BlockId::Pending).await.map_err(|e| format!("Failed to get pending block: {e}"))?;

    let hash_best = client.info().best_hash;
    let hash_current = block.parent_block_hash;
    let tmp = <B as BlockT>::Hash::from_str(&hash_current.to_string()).unwrap_or(Default::default());
    let number = block.block_number.ok_or("block number not found")? - 1;

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

    Ok(())
}

