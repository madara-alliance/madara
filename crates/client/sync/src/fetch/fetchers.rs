//! Contains the code required to fetch data from the network efficiently.
use core::fmt;
use core::time::Duration;

use anyhow::Context;
use futures::FutureExt;
use mc_block_import::{UnverifiedCommitments, UnverifiedFullBlock, UnverifiedHeader, UnverifiedPendingFullBlock};
use mc_db::MadaraBackend;
use mp_block::header::GasPrices;
use mp_chain_config::StarknetVersion;
use mp_class::class_update::{ClassUpdate, LegacyClassUpdate, SierraClassUpdate};
use mp_class::MISSED_CLASS_HASHES;
use mp_convert::{felt_to_u128, ToFelt};
use mp_receipt::TransactionReceipt;
use mp_transactions::{Transaction, MAIN_CHAIN_ID};
use mp_utils::{stopwatch_end, wait_or_graceful_shutdown, PerfStopwatch};
use starknet_api::core::ChainId;
use starknet_core::types::{ContractClass, MaybePendingBlockWithReceipts, StarknetError};
use starknet_providers::{Provider, ProviderError, SequencerGatewayProvider};
use starknet_types_core::felt::Felt;
use url::Url;

use crate::l2::L2SyncError;

use super::FetchError;

const MAX_RETRY: u32 = 15;
const BASE_DELAY: Duration = Duration::from_secs(1);

/// The configuration of the worker responsible for fetching new blocks and state updates from the
/// feeder.
#[derive(Clone, Debug)]
pub struct FetchConfig {
    /// The URL of the sequencer gateway.
    pub gateway: Url,
    /// The URL of the feeder gateway.
    pub feeder_gateway: Url,
    /// The ID of the chain served by the sequencer gateway.
    pub chain_id: ChainId,
    /// Whether to play a sound when a new block is fetched.
    pub sound: bool,
    /// Whether to check the root of the state update.
    pub verify: bool,
    /// The optional API_KEY to avoid rate limiting from the sequencer gateway.
    pub api_key: Option<String>,
    /// Polling interval.
    pub sync_polling_interval: Option<Duration>,
    /// Number of blocks to sync (for testing purposes).
    pub n_blocks_to_sync: Option<u64>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FetchBlockId {
    BlockN(u64),
    Pending,
}

impl FetchBlockId {
    pub fn block_n(self) -> Option<u64> {
        match self {
            FetchBlockId::BlockN(block_n) => Some(block_n),
            FetchBlockId::Pending => None,
        }
    }
}

impl fmt::Debug for FetchBlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BlockN(block) => write!(f, "#{}", block),
            Self::Pending => write!(f, "<pending>"),
        }
    }
}

impl From<FetchBlockId> for starknet_providers::sequencer::models::BlockId {
    fn from(value: FetchBlockId) -> Self {
        match value {
            FetchBlockId::BlockN(block_n) => starknet_providers::sequencer::models::BlockId::Number(block_n),
            FetchBlockId::Pending => starknet_providers::sequencer::models::BlockId::Pending,
        }
    }
}
impl From<FetchBlockId> for starknet_core::types::BlockId {
    fn from(value: FetchBlockId) -> Self {
        match value {
            FetchBlockId::BlockN(block_n) => starknet_core::types::BlockId::Number(block_n),
            FetchBlockId::Pending => starknet_core::types::BlockId::Tag(starknet_core::types::BlockTag::Pending),
        }
    }
}

pub async fn fetch_pending_block_and_updates(
    backend: &MadaraBackend,
    provider: &SequencerGatewayProvider,
) -> Result<UnverifiedPendingFullBlock, FetchError> {
    let block_id = FetchBlockId::Pending;

    let sw = PerfStopwatch::new();
    let (state_update, block) =
        retry(|| fetch_state_update_with_block(provider, block_id), MAX_RETRY, BASE_DELAY).await?;
    let class_update = fetch_class_updates(backend, &state_update, block_id, provider).await?;

    stopwatch_end!(sw, "fetching {:?}: {:?}", block_id);

    let block = starknet_core::types::MaybePendingBlockWithReceipts::try_from(block)
        .context("Converting the FGW format to starknet_types_core")?;

    let MaybePendingBlockWithReceipts::PendingBlock(block) = block else {
        return Err(anyhow::anyhow!("Fetched a pending block, got a closed one").into());
    };

    let (transactions, receipts) =
        block.transactions.into_iter().map(|t| (t.transaction.into(), t.receipt.into())).unzip();

    Ok(UnverifiedPendingFullBlock {
        header: UnverifiedHeader {
            parent_block_hash: Some(block.parent_hash),
            sequencer_address: block.sequencer_address,
            block_timestamp: block.timestamp,
            protocol_version: block.starknet_version.parse().context("Invalid starknet version")?,
            l1_gas_price: GasPrices {
                eth_l1_gas_price: felt_to_u128(&block.l1_gas_price.price_in_wei).context("Converting prices")?,
                strk_l1_gas_price: felt_to_u128(&block.l1_gas_price.price_in_fri).context("Converting prices")?,
                eth_l1_data_gas_price: felt_to_u128(&block.l1_data_gas_price.price_in_wei)
                    .context("Converting prices")?,
                strk_l1_data_gas_price: felt_to_u128(&block.l1_data_gas_price.price_in_fri)
                    .context("Converting prices")?,
            },
            l1_da_mode: block.l1_da_mode.into(),
        },
        state_diff: state_update.state_diff.into(),
        transactions,
        receipts,
        declared_classes: class_update.into_iter().map(Into::into).collect(),
    })
}

pub async fn fetch_block_and_updates(
    backend: &MadaraBackend,
    block_n: u64,
    provider: &SequencerGatewayProvider,
) -> Result<UnverifiedFullBlock, FetchError> {
    let block_id = FetchBlockId::BlockN(block_n);

    let sw = PerfStopwatch::new();
    let (state_update, block) =
        retry(|| fetch_state_update_with_block(provider, block_id), MAX_RETRY, BASE_DELAY).await?;
    let class_update = fetch_class_updates(backend, &state_update, block_id, provider).await?;

    stopwatch_end!(sw, "fetching {:?}: {:?}", block_id);

    // Verify against these commitments.
    let commitments = UnverifiedCommitments {
        // TODO: these commitments are wrong for mainnet from block 0 to unknown. We need to figure out
        // which blocks and handle the case directly in the block import crate.
        // transaction_commitment: Some(block.transaction_commitment.context("No transaction commitment")?),
        // event_commitment: Some(block.event_commitment.context("No event commitment")?),
        state_diff_commitment: None,
        receipt_commitment: None,
        global_state_root: Some(block.state_root.context("No state root")?),
        block_hash: Some(block.block_hash.context("No block hash")?),
        ..Default::default()
    };

    // let block = starknet_core::types::MaybePendingBlockWithReceipts::try_from(block)
    //     .context("Converting the FGW format to starknet_types_core")?;

    // let MaybePendingBlockWithReceipts::Block(block) = block else {
    //     return Err(anyhow::anyhow!("Fetched a closed block, got a pending one").into());
    // };

    Ok(UnverifiedFullBlock {
        unverified_block_number: Some(block.block_number.context("FGW should have a block number for closed blocks")?),
        header: UnverifiedHeader {
            parent_block_hash: Some(block.parent_block_hash),
            sequencer_address: block.sequencer_address.unwrap_or_default(),
            block_timestamp: block.timestamp,
            protocol_version: block
                .starknet_version
                .as_deref()
                .map(|v| v.parse().context("Invalid starknet version"))
                .unwrap_or(
                    StarknetVersion::try_from_mainnet_block_number(
                        block
                            .block_number
                            .context("A block number is needed to determine the missing Starknet version")?,
                    )
                    .ok_or(anyhow::anyhow!("Unable to determine the Starknet version")),
                )?,
            l1_gas_price: GasPrices {
                eth_l1_gas_price: felt_to_u128(&block.l1_gas_price.price_in_wei).context("Converting prices")?,
                strk_l1_gas_price: felt_to_u128(&block.l1_gas_price.price_in_fri).context("Converting prices")?,
                eth_l1_data_gas_price: felt_to_u128(&block.l1_data_gas_price.price_in_wei)
                    .context("Converting prices")?,
                strk_l1_data_gas_price: felt_to_u128(&block.l1_data_gas_price.price_in_fri)
                    .context("Converting prices")?,
            },
            l1_da_mode: block.l1_da_mode.into(),
        },
        state_diff: state_update.state_diff.into(),
        receipts: block
            .transaction_receipts
            .into_iter()
            .zip(&block.transactions)
            .map(|(receipt, tx)| TransactionReceipt::from_provider(receipt, tx))
            .collect(),
        transactions: block
            .transactions
            .into_iter()
            .map(Transaction::try_from)
            .collect::<Result<_, _>>()
            .context("Converting the FGW format")?,
        declared_classes: class_update.into_iter().map(Into::into).collect(),
        commitments,
    })
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

/// retrieves state update with block from Starknet sequencer in only one request
async fn fetch_state_update_with_block(
    provider: &SequencerGatewayProvider,
    block_id: FetchBlockId,
) -> Result<
    (starknet_providers::sequencer::models::StateUpdate, starknet_providers::sequencer::models::Block),
    ProviderError,
> {
    #[allow(deprecated)] // Sequencer-specific functions are deprecated. Use it via the Provider trait instead.
    let state_update_with_block = provider.get_state_update_with_block(block_id.into()).await?;

    Ok((state_update_with_block.state_update, state_update_with_block.block))
}

/// retrieves class updates from Starknet sequencer
async fn fetch_class_updates(
    backend: &MadaraBackend,
    state_update: &starknet_providers::sequencer::models::StateUpdate,
    block_id: FetchBlockId,
    provider: &SequencerGatewayProvider,
) -> anyhow::Result<Vec<ClassUpdate>> {
    let chain_id: Felt = backend.chain_config().chain_id.to_felt();

    // for blocks before 2597 on mainnet new classes are not declared in the state update
    // https://github.com/madara-alliance/madara/issues/233
    let legacy_classes: Vec<_> = if chain_id == MAIN_CHAIN_ID && block_id.block_n().is_some_and(|id| id < 2597) {
        let block_number = block_id.block_n().unwrap(); // Safe to unwrap because of the condition above
        MISSED_CLASS_HASHES.get(&block_number).cloned().unwrap_or_default()
    } else {
        state_update.state_diff.old_declared_contracts.clone()
    };

    let sierra_classes: Vec<_> = state_update
        .state_diff
        .declared_classes
        .iter()
        .map(|declared_class| (declared_class.class_hash, &declared_class.compiled_class_hash))
        .collect();

    let legacy_class_futures = legacy_classes.into_iter().map(|class_hash| {
        async move {
            let (class_hash, contract_class) =
                retry(|| fetch_class(class_hash, block_id, provider), 15, Duration::from_secs(1)).await?;

            let starknet_core::types::ContractClass::Legacy(contract_class) = contract_class else {
                return Err(L2SyncError::UnexpectedClassType { class_hash });
            };

            Ok::<_, L2SyncError>(ClassUpdate::Legacy(LegacyClassUpdate { class_hash, contract_class }))
        }
        .boxed()
    });

    let sierra_class_futures = sierra_classes.into_iter().map(|(class_hash, &compiled_class_hash)| {
        async move {
            let (class_hash, contract_class) =
                retry(|| fetch_class(class_hash, block_id, provider), 15, Duration::from_secs(1)).await?;

            let starknet_core::types::ContractClass::Sierra(contract_class) = contract_class else {
                return Err(L2SyncError::UnexpectedClassType { class_hash });
            };

            Ok::<_, L2SyncError>(ClassUpdate::Sierra(SierraClassUpdate {
                class_hash,
                contract_class,
                compiled_class_hash,
            }))
        }
        .boxed()
    });

    Ok(futures::future::try_join_all(legacy_class_futures.chain(sierra_class_futures)).await?)
}

/// Downloads a class definition from the Starknet sequencer. Note that because
/// of the current type hell we decided to deal with raw JSON data instead of starknet-providers `DeployedContract`.
async fn fetch_class(
    class_hash: Felt,
    block_id: FetchBlockId,
    provider: &SequencerGatewayProvider,
) -> Result<(Felt, ContractClass), ProviderError> {
    let contract_class = provider.get_class(starknet_core::types::BlockId::from(block_id), class_hash).await?;
    log::debug!("got the contract class here {:?}", contract_class);
    Ok((class_hash, contract_class))
}
