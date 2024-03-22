//! Contains the code required to fetch data from the feeder efficiently.
use std::pin::pin;
use std::str::FromStr;
use std::sync::{Arc, Mutex, RwLock};

use bitvec::order::Msb0;
use bitvec::view::AsBits;
use bonsai_trie::id::BasicId;
use bonsai_trie::BonsaiStorage;
use futures::{stream, StreamExt};
use itertools::Itertools;
use lazy_static::lazy_static;
use madara_runtime::opaque::{DBlockT, DHashT};
use mc_db::bonsai_db::BonsaiDb;
use mc_storage::OverrideHandle;
use mp_block::state_update::StateUpdateWrapper;
use mp_block::DeoxysBlock;
use mp_contract::class::{ClassUpdateWrapper, ContractClassData, ContractClassWrapper};
use mp_felt::Felt252Wrapper;
use mp_storage::StarknetStorageSchemaVersion;
use reqwest::Url;
use serde::Deserialize;
use sp_blockchain::HeaderBackend;
use sp_core::{H160, H256};
use sp_runtime::generic::{Block as RuntimeBlock, Header};
use sp_runtime::traits::{BlakeTwo256, UniqueSaturatedInto};
use sp_runtime::OpaqueExtrinsic;
use starknet_api::api_core::ClassHash;
use starknet_api::hash::StarkHash;
use starknet_core::types::{BlockId as BlockIdCore, PendingStateUpdate, StarknetError};
use starknet_ff::FieldElement;
use starknet_providers::sequencer::models as p;
use starknet_providers::sequencer::models::state_update::{DeclaredContract, DeployedContract};
use starknet_providers::sequencer::models::{BlockId, StateUpdate};
use starknet_providers::{Provider, ProviderError, SequencerGatewayProvider};
use starknet_types_core::hash::{Pedersen, Poseidon};
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinSet;
use tokio::time::{Duration, Instant};

use crate::commitments::lib::{build_commitment_state_diff, update_state_root};
use crate::CommandSink;

// TODO: add more error variants, which are more explicit
#[derive(Error, Debug)]
pub enum L2SyncError {
    #[error("provider error")]
    Provider(#[from] ProviderError),
    #[error("fetch retry limit exceeded")]
    FetchRetryLimit,
}

/// Contains the Starknet verified state on L2
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
    static ref STARKNET_HIGHEST_BLOCK_HASH_AND_NUMBER: RwLock<(FieldElement, u64)> = RwLock::new((FieldElement::default(), 0));
}

lazy_static! {
    /// Shared pending block data, using a RwLock to allow for concurrent reads and exclusive writes
    static ref STARKNET_PENDING_BLOCK: RwLock<Option<DeoxysBlock>> = RwLock::new(None);
}

lazy_static! {
    /// Shared pending state update, using RwLock to allow for concurrent reads and exclusive writes
    static ref STARKNET_PENDING_STATE_UPDATE: RwLock<Option<PendingStateUpdate>> = RwLock::new(None);
}

/// The configuration of the worker responsible for fetching new blocks and state updates from the
/// feeder.
#[derive(Clone, Debug)]
pub struct FetchConfig {
    /// The URL of the sequencer gateway.
    pub gateway: Url,
    /// The URL of the feeder gateway.
    pub feeder_gateway: Url,
    /// The ID of the chain served by the sequencer gateway.
    pub chain_id: starknet_ff::FieldElement,
    /// The number of tasks spawned to fetch blocks and state updates.
    pub workers: u32,
    /// Whether to play a sound when a new block is fetched.
    pub sound: bool,
    /// The L1 contract core address
    pub l1_core_address: H160,
}

/// The configuration of the senders responsible for sending blocks and state
/// updates from the feeder.
pub struct SenderConfig {
    /// Sender for dispatching fetched blocks.
    pub block_sender: Sender<DeoxysBlock>,
    /// Sender for dispatching fetched state updates.
    pub state_update_sender: Sender<StateUpdateWrapper>,
    /// Sender for dispatching fetched class hashes.
    pub class_sender: Sender<ClassUpdateWrapper>,
    /// The command sink used to notify the consensus engine that a new block
    /// should be created.
    pub command_sink: CommandSink,
    // Storage overrides for accessing stored classes
    pub overrides: Arc<OverrideHandle<RuntimeBlock<Header<u32, BlakeTwo256>, OpaqueExtrinsic>>>,
}

async fn fetch_block_and_updates<C>(
    block_n: u64,
    provider: &SequencerGatewayProvider,
    overrides: &Arc<OverrideHandle<RuntimeBlock<Header<u32, BlakeTwo256>, OpaqueExtrinsic>>>,
    client: &C,
) -> Result<(p::Block, StateUpdate, Vec<ContractClassData>), L2SyncError>
where
    C: HeaderBackend<DBlockT>,
{
    // retry loop
    const MAX_RETRY: usize = 15;
    for _ in 0..MAX_RETRY {
        log::debug!("fetch_block_and_updates {}", block_n);
        let block = fetch_block(provider, block_n);
        let state_update = fetch_state_and_class_update(provider, block_n, overrides, client);
        let (block, state_update) = tokio::join!(block, state_update);
        log::debug!("fetch_block_and_updates: done {block_n}");

        if matches!(
            block.as_ref().err().or(state_update.as_ref().err()),
            Some(L2SyncError::Provider(ProviderError::RateLimited))
        ) {
            continue; // retry api call
        }
        let (block, (state_update, class_update)) = (block?, state_update?);

        return Ok((block, state_update, class_update));
    }

    Err(L2SyncError::FetchRetryLimit)
}

/// Spawns workers to fetch blocks and state updates from the feeder.
pub async fn sync<C>(
    mut sender_config: SenderConfig,
    fetch_config: FetchConfig,
    first_block: u64,
    bonsai_contract: &Arc<Mutex<BonsaiStorage<BasicId, BonsaiDb, Pedersen>>>,
    bonsai_contract_storage: &Arc<Mutex<BonsaiStorage<BasicId, BonsaiDb, Pedersen>>>,
    bonsai_class: &Arc<Mutex<BonsaiStorage<BasicId, BonsaiDb, Poseidon>>>,
    client: Arc<C>,
) where
    C: HeaderBackend<DBlockT>,
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

async fn fetch_block(client: &SequencerGatewayProvider, block_number: u64) -> Result<p::Block, L2SyncError> {
    let block = client.get_block(BlockId::Number(block_number)).await?;

    Ok(block)
}

// FIXME: This is an artefact of an older version of the code when this was used to retrieve the
// head of the chain during initialization, but is since no longer used.

pub async fn fetch_apply_genesis_block(config: FetchConfig) -> Result<DeoxysBlock, String> {
    let client = SequencerGatewayProvider::new(config.gateway.clone(), config.feeder_gateway.clone(), config.chain_id);
    let block = client.get_block(BlockId::Number(0)).await.map_err(|e| format!("failed to get block: {e}"))?;

    Ok(crate::convert::block(block).await)
}

#[allow(clippy::too_many_arguments)]
async fn fetch_state_and_class_update<C>(
    provider: &SequencerGatewayProvider,
    block_number: u64,
    overrides: &Arc<OverrideHandle<RuntimeBlock<Header<u32, BlakeTwo256>, OpaqueExtrinsic>>>,
    client: &C,
) -> Result<(StateUpdate, Vec<ContractClassData>), L2SyncError>
where
    C: HeaderBackend<DBlockT>,
{
    // Children tasks need StateUpdate as an Arc, because of task spawn 'static requirement
    // We make an Arc, and then unwrap the StateUpdate out of the Arc
    let state_update = Arc::new(fetch_state_update(provider, block_number).await?);
    let class_update = fetch_class_update(provider, &state_update, overrides, block_number, client).await?;
    let state_update = Arc::try_unwrap(state_update).expect("arc should not be aliased");

    Ok((state_update, class_update))
}

/// retrieves state update from Starknet sequencer
async fn fetch_state_update(
    provider: &SequencerGatewayProvider,
    block_number: u64,
) -> Result<StateUpdate, L2SyncError> {
    let state_update = provider.get_state_update(BlockId::Number(block_number)).await?;

    Ok(state_update)
}

/// retrieves class updates from Starknet sequencer
async fn fetch_class_update<C>(
    provider: &SequencerGatewayProvider,
    state_update: &Arc<StateUpdate>,
    overrides: &Arc<OverrideHandle<RuntimeBlock<Header<u32, BlakeTwo256>, OpaqueExtrinsic>>>,
    block_number: u64,
    client: &C,
) -> Result<Vec<ContractClassData>, L2SyncError>
where
    C: HeaderBackend<DBlockT>,
{
    // defaults to downloading ALL classes if a substrate block hash could not be determined
    let missing_classes = match block_hash_substrate(client, block_number) {
        Some(block_hash_substrate) => fetch_missing_classes(state_update, overrides, block_hash_substrate),
        None => aggregate_classes(state_update),
    };

    let arc_provider = Arc::new(provider.clone());
    let mut task_set = missing_classes.into_iter().fold(JoinSet::new(), |mut set, class_hash| {
        let provider = Arc::clone(&arc_provider);
        let state_update = Arc::clone(state_update);
        let class_hash = *class_hash;
        set.spawn(async move { download_class(class_hash, block_hash_madara(&state_update), &provider).await });
        set
    });

    // WARNING: all class downloads will abort if even a single class fails to download.
    let mut classes = vec![];
    while let Some(res) = task_set.join_next().await {
        classes.push(res.expect("Join error")?);
        // No need to `abort_all()` the `task_set` in cast of errors, as dropping the `task_set`
        // will abort all the tasks.
    }

    Ok(classes)
}

/// Retrieves Madara block hash from state update
fn block_hash_madara(state_update: &StateUpdate) -> FieldElement {
    state_update.block_hash.unwrap()
}

/// Retrieves Substrate block hash from rpc client
fn block_hash_substrate<C>(client: &C, block_number: u64) -> Option<H256>
where
    C: HeaderBackend<DBlockT>,
{
    client
        .hash(UniqueSaturatedInto::unique_saturated_into(block_number))
        .unwrap()
        .map(|hash| H256::from_slice(hash.as_bits::<Msb0>().to_bitvec().as_raw_slice()))
}

/// Downloads a class definition from the Starknet sequencer. Note that because
/// of the current type hell this needs to be converted into a blockifier equivalent
async fn download_class(
    class_hash: FieldElement,
    block_hash: FieldElement,
    provider: &SequencerGatewayProvider,
) -> Result<ContractClassData, L2SyncError> {
    // log::info!("💾 Downloading class {class_hash:#x}");
    let core_class = provider.get_class(BlockIdCore::Hash(block_hash), class_hash).await?;

    // Core classes have to be converted into Blockifier classes to gain support
    // for Substrate [`Encode`] and [`Decode`] traits
    Ok(ContractClassData {
        // TODO: find a less roundabout way of converting from a Felt252Wrapper
        hash: ClassHash(Felt252Wrapper::from(class_hash).into()),
        // TODO: remove this expect when ContractClassWrapper::try_from does proper error handling using
        // thiserror
        contract_class: ContractClassWrapper::try_from(core_class).expect("converting contract class"),
    })
}

/// Filters out class declarations in the Starknet sequencer state update
/// and retains only those which are not stored in the local Substrate db.
fn fetch_missing_classes<'a>(
    state_update: &'a StateUpdate,
    overrides: &Arc<OverrideHandle<RuntimeBlock<Header<u32, BlakeTwo256>, OpaqueExtrinsic>>>,
    block_hash_substrate: H256,
) -> Vec<&'a FieldElement> {
    aggregate_classes(state_update)
        .into_iter()
        .filter(|class_hash| is_missing_class(overrides, block_hash_substrate, Felt252Wrapper::from(**class_hash)))
        .collect()
}

/// Retrieves all class hashes from state update. This includes newly deployed
/// contract class hashes, Sierra class hashes and Cairo class hashes
fn aggregate_classes(state_update: &StateUpdate) -> Vec<&FieldElement> {
    std::iter::empty()
        .chain(
            state_update
                .state_diff
                .deployed_contracts
                .iter()
                .map(|DeployedContract { address: _, class_hash }| class_hash),
        )
        .chain(
            state_update
                .state_diff
                .declared_classes
                .iter()
                .map(|DeclaredContract { class_hash, compiled_class_hash: _ }| class_hash),
        )
        .unique()
        .collect()
}

/// Check if a class is stored in the local Substrate db.
///
/// Since a change in class definition will result in a change in class hash,
/// this means we only need to check for class hashes in the db.
fn is_missing_class(
    overrides: &Arc<OverrideHandle<RuntimeBlock<Header<u32, BlakeTwo256>, OpaqueExtrinsic>>>,
    block_hash_substrate: H256,
    class_hash: Felt252Wrapper,
) -> bool {
    overrides
        .for_schema_version(&StarknetStorageSchemaVersion::Undefined)
        .contract_class_by_class_hash(block_hash_substrate, ClassHash::from(class_hash))
        .is_none()
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
pub fn verify_l2(
    block_number: u64,
    state_update: &StateUpdate,
    overrides: &Arc<OverrideHandle<RuntimeBlock<Header<u32, BlakeTwo256>, OpaqueExtrinsic>>>,
    bonsai_contract: &Arc<Mutex<BonsaiStorage<BasicId, BonsaiDb, Pedersen>>>,
    bonsai_contract_storage: &Arc<Mutex<BonsaiStorage<BasicId, BonsaiDb, Pedersen>>>,
    bonsai_class: &Arc<Mutex<BonsaiStorage<BasicId, BonsaiDb, Poseidon>>>,
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

async fn update_starknet_data<C>(provider: &SequencerGatewayProvider, client: &C) -> Result<(), String>
where
    C: HeaderBackend<DBlockT>,
{
    let block = provider.get_block(BlockId::Pending).await.map_err(|e| format!("Failed to get pending block: {e}"))?;

    let hash_best = client.info().best_hash;
    let hash_current = block.parent_block_hash;
    // Well howdy, seems like we can't convert a B::Hash to a FieldElement pa'tner,
    // fancy this instead? 🤠🔫
    let tmp = DHashT::from_str(&hash_current.to_string()).unwrap_or(Default::default());
    let number = block.block_number.ok_or("block number not found")? - 1;

    // all blocks have been synchronized, can store pending data
    if hash_best == tmp {
        let state_update = provider
            .get_state_update(BlockId::Pending)
            .await
            .map_err(|e| format!("Failed to get pending state update: {e}"))?;

        // Speaking about type conversion hell: 🔥
        *STARKNET_PENDING_BLOCK.write().expect("Failed to acquire write lock on STARKNET_PENDING_BLOCK") =
            Some(crate::convert::block(block).await);

        // This type conversion is evil and should not be necessary
        *STARKNET_PENDING_STATE_UPDATE.write().expect("Failed to aquire write lock on STARKNET_PENDING_STATE_UPDATE") =
            Some(crate::convert::state_update(state_update));
    }

    *STARKNET_HIGHEST_BLOCK_HASH_AND_NUMBER
        .write()
        .expect("Failed to acquire write lock on STARKNET_HIGHEST_BLOCK_HASH_AND_NUMBER") = (hash_current, number);

    Ok(())
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
