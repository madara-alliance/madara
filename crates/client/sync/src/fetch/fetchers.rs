//! Contains the code required to fetch data from the network efficiently.
use core::time::Duration;
use std::sync::Arc;

use itertools::Itertools;
use mc_storage::OverrideHandle;
use mp_block::DeoxysBlock;
use mp_contract::class::{ContractClassData, ContractClassWrapper};
use mp_felt::Felt252Wrapper;
use mp_storage::StarknetStorageSchemaVersion;
use mp_types::block::DBlockT;
use sp_blockchain::HeaderBackend;
use sp_core::{H160, H256};
use sp_runtime::generic::{Block as RuntimeBlock, Header};
use sp_runtime::traits::BlakeTwo256;
use sp_runtime::OpaqueExtrinsic;
use starknet_api::api_core::ClassHash;
use starknet_core::types::BlockId as BlockIdCore;
use starknet_ff::FieldElement;
use starknet_providers::sequencer::models as p;
use starknet_providers::sequencer::models::state_update::{DeclaredContract, DeployedContract};
use starknet_providers::sequencer::models::{BlockId, StateUpdate};
use starknet_providers::{Provider, ProviderError, SequencerGatewayProvider};
use tokio::task::JoinSet;
use url::Url;

use crate::l2::L2SyncError;
use crate::utility::{block_hash_deoxys, block_hash_substrate};

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
    /// Whether to check the root of the state update
    pub verify: bool,
}

pub async fn fetch_block(client: &SequencerGatewayProvider, block_number: u64) -> Result<p::Block, L2SyncError> {
    let block = client.get_block(BlockId::Number(block_number)).await?;

    Ok(block)
}

pub async fn fetch_block_and_updates<C>(
    block_n: u64,
    provider: &SequencerGatewayProvider,
    overrides: &Arc<OverrideHandle<RuntimeBlock<Header<u32, BlakeTwo256>, OpaqueExtrinsic>>>,
    client: &C,
) -> Result<(p::Block, StateUpdate, Vec<ContractClassData>), L2SyncError>
where
    C: HeaderBackend<DBlockT>,
{
    const MAX_RETRY: u32 = 15;
    let mut attempt = 0;
    let base_delay = Duration::from_secs(1);

    loop {
        log::debug!("fetch_block_and_updates {}", block_n);
        let block = fetch_block(provider, block_n);
        let state_update = fetch_state_and_class_update(provider, block_n, overrides, client);
        let (block, state_update) = tokio::join!(block, state_update);
        log::debug!("fetch_block_and_updates: done {block_n}");

        match block.as_ref().err().or(state_update.as_ref().err()) {
            Some(L2SyncError::Provider(ProviderError::RateLimited)) => {
                log::debug!("The fetching process has been rate limited, retrying in {:?} seconds", base_delay);
                attempt += 1;
                if attempt >= MAX_RETRY {
                    return Err(L2SyncError::FetchRetryLimit);
                }
                // Exponential backoff with a cap on the delay
                let delay = base_delay * 2_u32.pow(attempt - 1).min(6); // Cap to prevent overly long delays
                tokio::time::sleep(delay).await;
            }
            _ => {
                let (block, (state_update, class_update)) = (block?, state_update?);
                return Ok((block, state_update, class_update));
            }
        }
    }
}

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
        set.spawn(async move { fetch_class(class_hash, block_hash_deoxys(&state_update), &provider).await });
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

/// Downloads a class definition from the Starknet sequencer. Note that because
/// of the current type hell this needs to be converted into a blockifier equivalent
async fn fetch_class(
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
