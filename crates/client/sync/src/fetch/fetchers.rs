//! Contains the code required to fetch data from the network efficiently.
use core::fmt;
use core::time::Duration;
use std::collections::HashMap;

use anyhow::Context;
use dc_db::storage_updates::DbClassUpdate;
use dc_db::DeoxysBackend;
use dp_block::{
    header::GasPrices, BlockId, BlockTag, UnverifiedCommitments, UnverifiedFullBlock, UnverifiedFullPendingBlock,
    UnverifiedHeader,
};
use dp_class::DeclaredClass;
use dp_convert::felt_to_u128;
use dp_receipt::TransactionReceipt;
use dp_transactions::Transaction;
use dp_utils::{stopwatch_end, wait_or_graceful_shutdown, PerfStopwatch};
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
    /// Whether to check the root of the state update
    pub verify: bool,
    /// The optional API_KEY to avoid rate limiting from the sequencer gateway.
    pub api_key: Option<String>,
    /// Polling interval
    pub sync_polling_interval: Option<Duration>,
    /// Number of blocks to sync (for testing purposes)
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
    backend: &DeoxysBackend,
    provider: &SequencerGatewayProvider,
) -> Result<UnverifiedFullPendingBlock, FetchError> {
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

    Ok(UnverifiedFullPendingBlock {
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
        declared_classes: class_update
            .into_iter()
            .map(|c| DeclaredClass {
                class_hash: c.class_hash,
                contract_class: c.contract_class.into(),
                compiled_class_hash: c.compiled_class_hash,
            })
            .collect(),
    })
}

pub async fn fetch_block_and_updates(
    backend: &DeoxysBackend,
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
                .unwrap_or("0.0.0")
                .parse()
                .context("Invalid starknet version")?,
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
        declared_classes: class_update
            .into_iter()
            .map(|c| DeclaredClass {
                class_hash: c.class_hash,
                contract_class: c.contract_class.into(),
                compiled_class_hash: c.compiled_class_hash,
            })
            .collect(),
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
    backend: &DeoxysBackend,
    state_update: &starknet_providers::sequencer::models::StateUpdate,
    block_id: FetchBlockId,
    provider: &SequencerGatewayProvider,
) -> anyhow::Result<Vec<DbClassUpdate>> {
    let missing_classes: Vec<_> = std::iter::empty()
        .chain(
            state_update
                .state_diff
                .deployed_contracts
                .iter()
                .map(|deployed_contract| (deployed_contract.class_hash, &Felt::ZERO)),
        )
        .chain(state_update.state_diff.old_declared_contracts.iter().map(|&felt| (felt, &Felt::ZERO)))
        .chain(
            state_update
                .state_diff
                .declared_classes
                .iter()
                .map(|declared_class| (declared_class.class_hash, &declared_class.compiled_class_hash)),
        )
        .collect::<HashMap<_, _>>() // unique() by key
        .into_iter()
        .filter_map(|(class_hash, compiled_class_hash)| {
            match backend.contains_class(&BlockId::Tag(BlockTag::Latest), &class_hash) {
                Ok(false) => Some(Ok((class_hash, compiled_class_hash))),
                Ok(true) => None,
                Err(e) => Some(Err(e)),
            }
        })
        .collect::<Result<_, _>>()?;

    let classes = futures::future::try_join_all(missing_classes.into_iter().map(
        |(class_hash, &compiled_class_hash)| async move {
            // TODO(correctness): Skip what appears to be a broken Sierra class definition (quick fix)
            if class_hash
                != Felt::from_hex_unchecked("0x024f092a79bdff4efa1ec86e28fa7aa7d60c89b30924ec4dab21dbfd4db73698")
            {
                // Fetch the class definition in parallel, retrying up to 15 times for each class
                let (class_hash, contract_class) =
                    retry(|| fetch_class(class_hash, block_id, provider), 15, Duration::from_secs(1)).await?;
                Ok::<_, L2SyncError>(Some(DbClassUpdate { class_hash, contract_class, compiled_class_hash }))
            } else {
                Ok(None)
            }
        },
    ))
    .await?;

    Ok(classes.into_iter().flatten().collect())
}

/// Downloads a class definition from the Starknet sequencer. Note that because
/// of the current type hell we decided to deal with raw JSON data instead of starknet-providers `DeployedContract`.
async fn fetch_class(
    class_hash: Felt,
    block_id: FetchBlockId,
    provider: &SequencerGatewayProvider,
) -> Result<(Felt, ContractClass), ProviderError> {
    let contract_class = provider.get_class(starknet_core::types::BlockId::from(block_id), class_hash).await?;
    Ok((class_hash, contract_class))
}
