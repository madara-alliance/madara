//! Contains the code required to fetch data from the network efficiently.
use core::time::Duration;
use std::sync::Arc;

use dc_db::storage_handler::primitives::contract_class::{ContractClassData, ContractClassWrapper};
use dc_db::storage_handler::{DeoxysStorageError, StorageView};
use dc_db::DeoxysBackend;
use dp_block::DeoxysBlock;
use dp_convert::state_update::ToStateUpdateCore;
use dp_convert::ToStarkFelt;
use dp_transactions::{INTE_CHAIN_ID, MAIN_CHAIN_ID, TEST_CHAIN_ID};
use dp_utils::{stopwatch_end, wait_or_graceful_shutdown, PerfStopwatch};
use itertools::Itertools;
use reqwest::Client;
use starknet_api::core::ClassHash;
use starknet_core::types::{DeclaredClassItem, DeployedContractItem, StarknetError, StateUpdate};
use starknet_providers::sequencer::models::{self as p, BlockId};
use starknet_providers::{ProviderError, SequencerGatewayProvider};
use url::Url;

use starknet_types_core::felt::Felt;

use crate::l2::L2SyncError;

/// The configuration of the worker responsible for fetching new blocks and state updates from the
/// feeder.
#[derive(Clone, Debug)]
pub struct FetchConfig {
    /// The URL of the sequencer gateway.
    pub gateway: Url,
    /// The URL of the feeder gateway.
    pub feeder_gateway: Url,
    /// The ID of the chain served by the sequencer gateway.
    pub chain_id: Felt,
    /// Whether to play a sound when a new block is fetched.
    pub sound: bool,
    /// The L1 contract core address
    pub l1_core_address: dp_block::H160,
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
    backend: &DeoxysBackend,
    block_n: u64,
    provider: Arc<SequencerGatewayProvider>,
    chain_id: Felt,
) -> Result<L2BlockAndUpdates, L2SyncError> {
    const MAX_RETRY: u32 = 15;
    let base_delay = Duration::from_secs(1);

    let sw = PerfStopwatch::new();
    let (state_update, block) =
        retry(|| fetch_state_update_with_block(&provider, block_n), MAX_RETRY, base_delay).await?;
    let class_update = fetch_class_update(backend, &state_update, block_n, chain_id).await?;

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
                    _ => log::warn!("The provider has returned an error: {}, retrying in {:?}", err, delay),
                }
                if wait_or_graceful_shutdown(tokio::time::sleep(delay)).await.is_none() {
                    return Err(ProviderError::StarknetError(StarknetError::BlockNotFound));
                    // :/
                }
            }
        }
    }
}

pub async fn fetch_apply_genesis_block(config: FetchConfig, chain_id: Felt) -> Result<DeoxysBlock, String> {
    let client = SequencerGatewayProvider::new(config.gateway.clone(), config.feeder_gateway.clone(), config.chain_id);
    let client = match &config.api_key {
        Some(api_key) => client.with_header("X-Throttling-Bypass".to_string(), api_key.clone()),
        None => client,
    };
    let block = client.get_block(BlockId::Number(0)).await.map_err(|e| format!("failed to get block: {e}"))?;

    Ok(crate::convert::convert_block(block, chain_id).expect("invalid genesis block"))
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
    backend: &DeoxysBackend,
    state_update: &StateUpdate,
    block_number: u64,
    chain_id: Felt,
) -> Result<Vec<ContractClassData>, L2SyncError> {
    let missing_classes: Vec<&Felt> = std::iter::empty()
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
        .filter_map(|class_hash| match is_missing_class(backend, class_hash) {
            Ok(true) => Some(Ok(class_hash)),
            Ok(false) => None,
            Err(e) => Some(Err(e)),
        })
        .collect::<Result<Vec<_>, _>>()?;

    let classes = futures::future::try_join_all(missing_classes.into_iter().map(|class_hash| async {
        let class_hash = *class_hash;
        // TODO(correctness): Skip what appears to be a broken Sierra class definition (quick fix)
        if class_hash != Felt::from_hex("0x024f092a79bdff4efa1ec86e28fa7aa7d60c89b30924ec4dab21dbfd4db73698").unwrap() {
            // Fetch the class definition in parallel, retrying up to 15 times for each class
            retry(|| fetch_class(class_hash, block_number, chain_id), 15, Duration::from_secs(1)).await.map(Some)
        } else {
            Ok(None)
        }
    }))
    .await?;

    Ok(classes.into_iter().flatten().collect())
}

/// This method is used to fetch a class definition from the sequencer gateway in it's raw format for an easier conversion.
pub async fn raw_get_class_by_hash(
    gateway_url: &str,
    class_hash: &str,
    block_number: u64,
) -> Result<serde_json::Value, ProviderError> {
    let client = Client::new();
    let url = format!(
        "{}/feeder_gateway/get_class_by_hash?classHash={}&blockNumber={}",
        gateway_url, class_hash, block_number
    );
    let response = client.get(&url).send().await.map_err(|_| ProviderError::ArrayLengthMismatch)?;
    let json: serde_json::Value = response.json().await.map_err(|_| ProviderError::ArrayLengthMismatch)?;
    Ok(json)
}

/// Downloads a class definition from the Starknet sequencer. Note that because
/// of the current type hell we decided to deal with raw JSON data instead of starknet-providers `DeployedContract`.
async fn fetch_class(class_hash: Felt, block_number: u64, chain_id: Felt) -> Result<ContractClassData, ProviderError> {
    // Configuring custom provider to fetch raw json classe definitions
    let url = match chain_id {
        id if id == MAIN_CHAIN_ID => "https://alpha-mainnet.starknet.io",
        id if id == TEST_CHAIN_ID => "https://alpha-sepolia.starknet.io",
        id if id == INTE_CHAIN_ID => "https://external.integration.starknet.io",
        _ => return Err(ProviderError::StarknetError(StarknetError::ClassHashNotFound)), // Set a more appropriate error here
    };

    let core_class = raw_get_class_by_hash(url, &class_hash.to_hex_string(), block_number).await?;
    Ok(ContractClassData {
        hash: ClassHash(class_hash.to_stark_felt()),
        contract_class: ContractClassWrapper::try_from((core_class, class_hash)).expect("converting contract class"),
    })
}

/// Check if a class is stored in the db.
///
/// Since a change in class definition will result in a change in class hash,
/// this means we only need to check for class hashes in the db.
fn is_missing_class(backend: &DeoxysBackend, class_hash: &Felt) -> Result<bool, DeoxysStorageError> {
    let class_hash = ClassHash(class_hash.to_stark_felt());
    backend.contract_class_data().contains(&class_hash).map(|x| !x)
}
