//! Contains the code required to fetch data from the network efficiently.
use core::time::Duration;
use std::sync::Arc;

use itertools::Itertools;
use mc_db::storage_handler::primitives::contract_class::{ContractClassData, ContractClassWrapper};
use mc_db::storage_handler::{self, DeoxysStorageError, StorageView};
use mp_block::DeoxysBlock;
use mp_convert::state_update::ToStateUpdateCore;
use sp_core::H160;
use starknet_api::core::ClassHash;
use starknet_api::hash::StarkFelt;
use starknet_core::types::{
    BlockId as BlockIdCore, DeclaredClassItem, DeployedContractItem, StarknetError, StateUpdate,
};
use starknet_ff::FieldElement;
use starknet_providers::sequencer::models::{self as p, BlockId};
use starknet_providers::{Provider, ProviderError, SequencerGatewayProvider};
use tokio::task::JoinSet;
use url::Url;

use crate::l2::L2SyncError;
use crate::stopwatch_end;
use crate::utils::PerfStopwatch;

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
    /// The optional API_KEY to avoid rate limiting from the sequencer gateway.
    pub api_key: Option<String>,
    /// Polling interval
    pub sync_polling_interval: Option<Duration>,
    /// Number of blocks to sync (for testing purposes)
    pub n_blocks_to_sync: Option<u64>,
}

pub struct L2BlockAndUpdates {
    pub block_n: u64,
    pub block: p::Block,
    pub state_update: StateUpdate,
    pub class_update: Vec<ContractClassData>,
}

pub async fn fetch_block_and_updates(
    block_n: u64,
    provider: Arc<SequencerGatewayProvider>,
) -> Result<L2BlockAndUpdates, L2SyncError> {
    const MAX_RETRY: u32 = 15;
    let base_delay = Duration::from_secs(1);

    let sw = PerfStopwatch::new();
    let (state_update, block) =
        retry(|| fetch_state_update_with_block(&provider, block_n), MAX_RETRY, base_delay).await?;
    let class_update = fetch_class_update(&provider, &state_update, block_n).await?;

    stopwatch_end!(sw, "fetching {}: {:?}", block_n);
    Ok(L2BlockAndUpdates { block_n, block, state_update, class_update })
}

async fn retry<F, Fut, T>(mut f: F, max_retries: u32, base_delay: Duration) -> Result<T, ProviderError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, ProviderError>>,
{
    let mut attempt = 0;
    loop {
        match f().await {
            Ok(res) => return Ok(res),
            Err(ProviderError::StarknetError(StarknetError::BlockNotFound)) => {
                break Err(ProviderError::StarknetError(StarknetError::BlockNotFound));
            }
            Err(err) => {
                let delay = base_delay * 2_u32.pow(attempt).min(6); // Cap to prevent overly long delays
                attempt += 1;
                if attempt > max_retries {
                    break Err(err);
                }
                match err {
                    ProviderError::RateLimited => {
                        log::info!("The fetching process has been rate limited, retrying in {:?}", delay)
                    }
                    _ => log::info!("The provider has returned an error: {}, retrying in {:?}", err, delay),
                }
                tokio::time::sleep(delay).await;
            }
        }
    }
}

pub async fn fetch_apply_genesis_block(config: FetchConfig) -> Result<DeoxysBlock, String> {
    let client = SequencerGatewayProvider::new(config.gateway.clone(), config.feeder_gateway.clone(), config.chain_id);
    let client = match &config.api_key {
        Some(api_key) => client.with_header("X-Throttling-Bypass".to_string(), api_key.clone()),
        None => client,
    };
    let block = client.get_block(BlockId::Number(0)).await.map_err(|e| format!("failed to get block: {e}"))?;

    Ok(crate::convert::convert_block(block).expect("invalid genesis block"))
}

/// retrieves state update with block from Starknet sequencer in only one request
async fn fetch_state_update_with_block(
    provider: &SequencerGatewayProvider,
    block_number: u64,
) -> Result<(StateUpdate, p::Block), ProviderError> {
    let state_update_with_block = provider.get_state_update_with_block(BlockId::Number(block_number)).await?;

    Ok((state_update_with_block.state_update.to_state_update_core(), state_update_with_block.block))
}

/// retrieves class updates from Starknet sequencer
async fn fetch_class_update(
    provider: &SequencerGatewayProvider,
    state_update: &StateUpdate,
    block_number: u64,
) -> Result<Vec<ContractClassData>, L2SyncError> {
    let missing_classes: Vec<&FieldElement> = std::iter::empty()
        .chain(
            state_update
                .state_diff
                .deployed_contracts
                .iter()
                .map(|DeployedContractItem { address: _, class_hash }| class_hash),
        )
        .chain(
            state_update
                .state_diff
                .declared_classes
                .iter()
                .map(|DeclaredClassItem { class_hash, compiled_class_hash: _ }| class_hash),
        )
        .chain(state_update.state_diff.deprecated_declared_classes.iter())
        .unique()
        .filter_map(|class_hash| match is_missing_class(class_hash) {
            Ok(true) => Some(Ok(class_hash)),
            Ok(false) => None,
            Err(e) => Some(Err(e)),
        })
        .collect::<Result<Vec<_>, _>>()?;

    let arc_provider = Arc::new(provider.clone());

    let mut task_set = missing_classes.into_iter().fold(JoinSet::new(), |mut set, class_hash| {
        let provider = Arc::clone(&arc_provider);
        let class_hash = *class_hash;
        // Skip what appears to be a broken Sierra class definition (quick fix)
        if class_hash
            != FieldElement::from_hex_be("0x024f092a79bdff4efa1ec86e28fa7aa7d60c89b30924ec4dab21dbfd4db73698").unwrap()
        {
            // Fetch the class definition in parallel, retrying up to 15 times for each class
            set.spawn(async move {
                retry(|| fetch_class(class_hash, block_number, &provider), 15, Duration::from_secs(1)).await
            });
        }
        set
    });

    // WARNING: all class downloads will abort if even a single class fails to download.
    let mut classes = vec![];
    while let Some(res) = task_set.join_next().await {
        classes.push(res.expect("Join error")?);
    }

    Ok(classes)
}

/// Downloads a class definition from the Starknet sequencer. Note that because
/// of the current type hell this needs to be converted into a blockifier equivalent
async fn fetch_class(
    class_hash: FieldElement,
    block_number: u64,
    provider: &SequencerGatewayProvider,
) -> Result<ContractClassData, ProviderError> {
    let core_class = provider.get_class(BlockIdCore::Number(block_number), class_hash).await?;
    Ok(ContractClassData {
        hash: ClassHash(StarkFelt(class_hash.to_bytes_be())),
        contract_class: ContractClassWrapper::try_from(core_class).expect("converting contract class"),
    })
}

/// Check if a class is stored in the local Substrate db.
///
/// Since a change in class definition will result in a change in class hash,
/// this means we only need to check for class hashes in the db.
fn is_missing_class(class_hash: &FieldElement) -> Result<bool, DeoxysStorageError> {
    let class_hash = ClassHash(StarkFelt(class_hash.to_bytes_be()));
    // TODO: return the db error instead of unwrapping
    storage_handler::contract_class_data().contains(&class_hash).map(|x| !x)
}
