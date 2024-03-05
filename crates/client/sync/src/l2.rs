//! Contains the code required to fetch data from the feeder efficiently.
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use itertools::Itertools;
use mc_db::BonsaiDbs;
use mc_storage::OverrideHandle;
use mp_block::state_update::StateUpdateWrapper;
use mp_contract::class::{ClassUpdateWrapper, ContractClassData, ContractClassWrapper};
use mp_felt::Felt252Wrapper;
use mp_storage::StarknetStorageSchemaVersion;
use reqwest::Url;
use serde::Deserialize;
use sp_core::H256;
use sp_runtime::generic::{Block, Header};
use sp_runtime::traits::{BlakeTwo256, Block as BlockT};
use sp_runtime::OpaqueExtrinsic;
use starknet_api::api_core::ClassHash;
use starknet_api::hash::StarkHash;
use starknet_core::types::BlockId as BlockIdCore;
use starknet_ff::FieldElement;
use starknet_providers::sequencer::models::state_update::{DeclaredContract, DeployedContract};
use starknet_providers::sequencer::models::{BlockId, StateUpdate};
use starknet_providers::{Provider, SequencerGatewayProvider};
use tokio::sync::mpsc::Sender;
use tokio::task::JoinSet;

use crate::commitments::lib::{build_commitment_state_diff, update_state_root};
use crate::utility::{get_block_hash_by_number, update_highest_block_hash_and_number};
use crate::CommandSink;

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

use lazy_static::lazy_static;

// TODO: find a better place to store this
lazy_static! {
    /// Store the configuration globally
    static ref CONFIG: Arc<Mutex<FetchConfig>> = Arc::new(Mutex::new(FetchConfig::default()));
}

lazy_static! {
    /// Shared latest block number and hash of chain
    pub static ref STARKNET_HIGHEST_BLOCK_HASH_AND_NUMBER: Arc<Mutex<(FieldElement, u64)>> = Arc::new(Mutex::new((FieldElement::default(), 0)));
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
}

impl Default for FetchConfig {
    fn default() -> Self {
        FetchConfig {
            // Provide default values for each field of FetchConfig
            gateway: Url::parse("http://default-gateway-url.com").unwrap(),
            feeder_gateway: Url::parse("http://default-feeder-gateway-url.com").unwrap(),
            chain_id: starknet_ff::FieldElement::default(), // Adjust as necessary
            workers: 4,
            sound: false,
        }
    }
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

// TODO: find a better place to store this
/// Stores a madara block hash and it's associated substrate hash.
pub struct BlockHashEquivalence {
    pub madara: FieldElement,
    pub substrate: Option<H256>,
}

impl BlockHashEquivalence {
    async fn new(state_update: &StateUpdate, block_number: u64, rpc_port: u16) -> Self {
        // TODO: use an actual Substrate client to convert from Madara to Substrate block hash
        let block_hash_madara = state_update.block_hash.unwrap();
        let block_hash_substrate = &get_block_hash_by_number(rpc_port, block_number).await;

        // WARNING: might causes issues related to eRFC 2497 (https://github.com/rust-lang/rust/issues/53667)
        if block_number > 0 && let Some(block_hash_substrate) = block_hash_substrate {
            BlockHashEquivalence {
                madara: block_hash_madara,
                substrate: Some(H256::from_str(block_hash_substrate).unwrap()),
            }
        } else {
            BlockHashEquivalence {
                madara: block_hash_madara,
                substrate: None,
            }
        }
    }
}

/// Spawns workers to fetch blocks and state updates from the feeder.
pub async fn sync<B: BlockT>(
    mut sender_config: SenderConfig,
    config: FetchConfig,
    start_at: u64,
    rpc_port: u16,
    backend: Arc<mc_db::Backend<B>>,
) {
    update_config(&config);
    let SenderConfig { block_sender, state_update_sender, class_sender, command_sink, overrides } = &mut sender_config;
    let client = SequencerGatewayProvider::new(config.gateway.clone(), config.feeder_gateway.clone(), config.chain_id);
    let bonsai_dbs = BonsaiDbs {
        contract: Arc::clone(backend.bonsai_contract()),
        class: Arc::clone(backend.bonsai_class()),
        storage: Arc::clone(backend.bonsai_storage()),
    };
    let mut current_block_number = start_at;
    let mut last_block_hash = None;
    let mut got_block = false;
    let mut got_state_update = false;
    let mut last_update_highest_block = tokio::time::Instant::now() - Duration::from_secs(20);
    if current_block_number == 0 {
        let _ = fetch_genesis_state_update(&client, bonsai_dbs.clone()).await;
    }
    loop {
        if last_update_highest_block.elapsed() > Duration::from_secs(20) {
            last_update_highest_block = tokio::time::Instant::now();
            if let Err(e) = update_highest_block_hash_and_number(&client).await {
                eprintln!("Failed to update highest block hash and number: {}", e);
            }
        }
        let (block, state_update) = match (got_block, got_state_update) {
            (false, false) => {
                let block = fetch_block(&client, block_sender, current_block_number);
                let state_update = fetch_state_and_class_update(
                    &client,
                    Arc::clone(overrides),
                    state_update_sender,
                    class_sender,
                    current_block_number,
                    rpc_port,
                    bonsai_dbs.clone(),
                );
                tokio::join!(block, state_update)
            }
            (false, true) => (fetch_block(&client, block_sender, current_block_number).await, Ok(())),
            (true, false) => (
                Ok(()),
                fetch_state_and_class_update(
                    &client,
                    Arc::clone(overrides),
                    state_update_sender,
                    class_sender,
                    current_block_number,
                    rpc_port,
                    bonsai_dbs.clone(),
                )
                .await,
            ),
            (true, true) => unreachable!(),
        };

        got_block = got_block || block.is_ok();
        got_state_update = got_state_update || state_update.is_ok();

        match (block, state_update) {
            (Ok(()), Ok(())) => match create_block(command_sink, &mut last_block_hash).await {
                Ok(()) => {
                    current_block_number += 1;
                    got_block = false;
                    got_state_update = false;
                }
                Err(e) => {
                    eprintln!("Failed to create block: {}", e);
                    return;
                }
            },
            (Err(a), Ok(())) => {
                eprintln!("Failed to fetch block {}: {}", current_block_number, a);
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
            (_, Err(b)) => {
                eprintln!("Failed to fetch state update {}: {}", current_block_number, b);
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        }
    }
}

async fn fetch_block(
    client: &SequencerGatewayProvider,
    block_sender: &Sender<mp_block::Block>,
    block_number: u64,
) -> Result<(), String> {
    let block =
        client.get_block(BlockId::Number(block_number)).await.map_err(|e| format!("failed to get block: {e}"))?;

    let block_conv = crate::convert::block(block).await;
    block_sender.send(block_conv).await.map_err(|e| format!("failed to dispatch block: {e}"))?;

    Ok(())
}

pub async fn fetch_genesis_block(config: FetchConfig) -> Result<mp_block::Block, String> {
    let client = SequencerGatewayProvider::new(config.gateway.clone(), config.feeder_gateway.clone(), config.chain_id);
    let block = client.get_block(BlockId::Number(0)).await.map_err(|e| format!("failed to get block: {e}"))?;

    Ok(crate::convert::block(block).await)
}

async fn fetch_state_and_class_update<B: BlockT>(
    provider: &SequencerGatewayProvider,
    overrides: Arc<OverrideHandle<Block<Header<u32, BlakeTwo256>, OpaqueExtrinsic>>>,
    state_update_sender: &Sender<StateUpdateWrapper>,
    class_sender: &Sender<ClassUpdateWrapper>,
    block_number: u64,
    rpc_port: u16,
    bonsai_dbs: BonsaiDbs<B>,
) -> Result<(), String> {
    let state_update = fetch_state_update(provider, block_number, bonsai_dbs).await?;
    let class_update = fetch_class_update(provider, &state_update, overrides, block_number, rpc_port).await?;

    // Now send state_update, which moves it. This will be received
    // by QueryBlockConsensusDataProvider in deoxys/crates/node/src/service.rs
    state_update_sender
        .send(StateUpdateWrapper::from(state_update))
        .await
        .map_err(|e| format!("failed to dispatch state update: {e}"))?;

    // do the same to class update
    class_sender
        .send(ClassUpdateWrapper(class_update))
        .await
        .map_err(|e| format!("failed to dispatch class update: {e}"))?;

    Ok(())
}

/// retrieves state update from Starknet sequencer
async fn fetch_state_update<B: BlockT>(
    provider: &SequencerGatewayProvider,
    block_number: u64,
    bonsai_dbs: BonsaiDbs<B>,
) -> Result<StateUpdate, String> {
    let state_update = provider
        .get_state_update(BlockId::Number(block_number))
        .await
        .map_err(|e| format!("failed to get state update: {e}"))?;

    verify_l2(block_number, &state_update, bonsai_dbs).await?;

    Ok(state_update)
}

async fn fetch_genesis_state_update<B: BlockT>(
    provider: &SequencerGatewayProvider,
    bonsai_dbs: BonsaiDbs<B>,
) -> Result<StateUpdate, String> {
    let state_update =
        provider.get_state_update(BlockId::Number(0)).await.map_err(|e| format!("failed to get state update: {e}"))?;

    verify_l2(0, &state_update, bonsai_dbs).await?;

    Ok(state_update)
}

/// retrieves class updates from Starknet sequencer
async fn fetch_class_update(
    provider: &SequencerGatewayProvider,
    state_update: &StateUpdate,
    overrides: Arc<OverrideHandle<Block<Header<u32, BlakeTwo256>, OpaqueExtrinsic>>>,
    block_number: u64,
    rpc_port: u16,
) -> Result<Vec<ContractClassData>, String> {
    // defaults to downloading ALL classes if a substrate block hash could not be determined
    let block_hash = BlockHashEquivalence::new(state_update, block_number - 1, rpc_port).await;
    let missing_classes = match block_hash.substrate {
        Some(block_hash_substrate) => fetch_missing_classes(state_update, overrides, block_hash_substrate),
        None => aggregate_classes(state_update),
    };

    let arc_provider = Arc::new(provider.clone());
    let mut task_set = missing_classes.into_iter().fold(JoinSet::new(), |mut set, class_hash| {
        set.spawn(download_class(*class_hash, block_hash.madara, Arc::clone(&arc_provider)));
        set
    });

    // WARNING: all class downloads will abort if even a single class fails to download.
    let mut classes = vec![];
    while let Some(res) = task_set.join_next().await {
        match res {
            Ok(result) => match result {
                Ok(contract) => classes.push(contract),
                Err(e) => {
                    task_set.abort_all();
                    return Err(e.to_string());
                }
            },
            Err(e) => {
                task_set.abort_all();
                return Err(e.to_string());
            }
        }
    }

    Ok(classes)
}

/// Downloads a class definition from the Starknet sequencer. Note that because
/// of the current type hell this needs to be converted into a blockifier equivalent
async fn download_class(
    class_hash: FieldElement,
    block_hash: FieldElement,
    provider: Arc<SequencerGatewayProvider>,
) -> anyhow::Result<ContractClassData> {
    // log::info!("💾 Downloading class {class_hash:#x}");
    let core_class = provider.get_class(BlockIdCore::Hash(block_hash), class_hash).await?;

    // Core classes have to be converted into Blockifier classes to gain support
    // for Substrate [`Encode`] and [`Decode`] traits
    Ok(ContractClassData {
        // TODO: find a less roundabout way of converting from a Felt252Wrapper
        hash: ClassHash(Felt252Wrapper::from(class_hash).into()),
        contract_class: ContractClassWrapper::try_from(core_class)?,
    })
}

/// Filters out class declarations in the Starknet sequencer state update
/// and retains only those which are not stored in the local Substrate db.
fn fetch_missing_classes(
    state_update: &StateUpdate,
    overrides: Arc<OverrideHandle<Block<Header<u32, BlakeTwo256>, OpaqueExtrinsic>>>,
    block_hash_substrate: H256,
) -> Vec<&FieldElement> {
    aggregate_classes(state_update)
        .into_iter()
        .filter(|class_hash| {
            is_missing_class(Arc::clone(&overrides), block_hash_substrate, Felt252Wrapper::from(**class_hash))
        })
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
    overrides: Arc<OverrideHandle<Block<Header<u32, BlakeTwo256>, OpaqueExtrinsic>>>,
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
    {
        let mut last_state_update = STARKNET_STATE_UPDATE.lock().expect("failed to lock STARKNET_STATE_UPDATE");
        *last_state_update = state_update.clone();
    }
}

/// Verify and update the L2 state according to the latest state update
pub async fn verify_l2<B: BlockT>(
    block_number: u64,
    state_update: &StateUpdate,
    bonsai_dbs: BonsaiDbs<B>,
) -> Result<(), String> {
    let state_update_wrapper = StateUpdateWrapper::from(state_update);

    let csd = build_commitment_state_diff(state_update_wrapper.clone());

    // Main l2 sync bottleneck HERE!
    let state_root =
        update_state_root(csd, bonsai_dbs).await.map_err(|e| format!("Failed to update state root: {e}"))?;

    let block_hash = state_update.block_hash.expect("Block hash not found in state update");

    update_l2(L2StateUpdate {
        block_number,
        global_root: state_root.into(),
        block_hash: Felt252Wrapper::from(block_hash).into(),
    });

    Ok(())
}

pub fn get_highest_block_hash_and_number() -> (FieldElement, u64) {
    *STARKNET_HIGHEST_BLOCK_HASH_AND_NUMBER.lock().expect("failed to lock STARKNET_HIGHEST_BLOCK_HASH_AND_NUMBER")
}

fn update_config(config: &FetchConfig) {
    let last_config = CONFIG.clone();
    let mut new_config = last_config.lock().unwrap();
    *new_config = config.clone();
}

pub fn get_config() -> FetchConfig {
    CONFIG.lock().unwrap().clone()
}
