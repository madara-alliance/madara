//! Contains the code required to fetch data from the network efficiently.
use core::time::Duration;
use std::sync::Arc;

use dc_db::storage_handler::{DeoxysStorageError, StorageView};
use dc_db::DeoxysBackend;
use dp_block::DeoxysBlock;
use dp_convert::ToStateUpdateCore;
use dp_utils::{stopwatch_end, wait_or_graceful_shutdown, PerfStopwatch};
use itertools::Itertools;
use starknet_core::types::{ContractClass, DeclaredClassItem, DeployedContractItem, StarknetError, StateUpdate};
use starknet_providers::sequencer::models::{self as p};
use starknet_providers::{Provider, ProviderError, SequencerGatewayProvider};
use starknet_types_core::felt::Felt;
use url::Url;

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
    /// Disable l1 sync
    pub sync_l1_disabled: bool,
}

pub struct L2BlockAndUpdates {
    pub block_n: u64,
    pub block: p::Block,
    pub state_update: StateUpdate,
    pub class_update: Vec<(Felt, ContractClass)>,
}

pub async fn fetch_block_and_updates(
    backend: &DeoxysBackend,
    block_n: u64,
    provider: Arc<SequencerGatewayProvider>,
) -> Result<L2BlockAndUpdates, L2SyncError> {
    const MAX_RETRY: u32 = 15;
    let base_delay = Duration::from_secs(1);

    let sw = PerfStopwatch::new();
    let (state_update, block) =
        retry(|| fetch_state_update_with_block(&provider, block_n), MAX_RETRY, base_delay).await?;
    let class_update = fetch_class_update(backend, &state_update, block_n, &provider).await?;

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
    let block = client.get_block(p::BlockId::Number(0)).await.map_err(|e| format!("failed to get block: {e}"))?;

    Ok(crate::convert::convert_and_verify_block(block, chain_id).expect("invalid genesis block"))
}

/// retrieves state update with block from Starknet sequencer in only one request
async fn fetch_state_update_with_block(
    provider: &SequencerGatewayProvider,
    block_number: u64,
) -> Result<(StateUpdate, p::Block), ProviderError> {
    let state_update_with_block = provider.get_state_update_with_block(p::BlockId::Number(block_number)).await?;

    Ok((state_update_with_block.state_update.to_state_update_core(), state_update_with_block.block))
}

/// retrieves class updates from Starknet sequencer
async fn fetch_class_update(
    backend: &DeoxysBackend,
    state_update: &StateUpdate,
    block_number: u64,
    provider: &SequencerGatewayProvider,
) -> Result<Vec<(Felt, ContractClass)>, L2SyncError> {
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
            retry(|| fetch_class(class_hash, block_number, provider), 15, Duration::from_secs(1)).await.map(Some)
        } else {
            Ok(None)
        }
    }))
    .await?;

    Ok(classes.into_iter().flatten().collect())
}

/// Downloads a class definition from the Starknet sequencer. Note that because
/// of the current type hell we decided to deal with raw JSON data instead of starknet-providers `DeployedContract`.
async fn fetch_class(
    class_hash: Felt,
    block_number: u64,
    provider: &SequencerGatewayProvider,
) -> Result<(Felt, ContractClass), ProviderError> {
    let contract_class = provider.get_class(starknet_core::types::BlockId::Number(block_number), class_hash).await?;
    Ok((class_hash, contract_class))
}

/// Check if a class is stored in the db.
///
/// Since a change in class definition will result in a change in class hash,
/// this means we only need to check for class hashes in the db.
fn is_missing_class(backend: &DeoxysBackend, class_hash: &Felt) -> Result<bool, DeoxysStorageError> {
    backend.contract_class_data().contains(class_hash).map(|x| !x)
}
