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
) -> Result<Option<UnverifiedPendingFullBlock>, FetchError> {
    let block_id = FetchBlockId::Pending;

    let sw = PerfStopwatch::new();
    let (state_update, block) =
        retry(|| fetch_state_update_with_block(provider, block_id), MAX_RETRY, BASE_DELAY).await?;

    let block = starknet_core::types::MaybePendingBlockWithReceipts::try_from(block)
        .context("Converting the FGW format to starknet_types_core")?;

    let block = match block {
        MaybePendingBlockWithReceipts::Block(block) => {
            // HACK: Apparently the FGW sometimes returns a closed block when fetching the pending block. Interesting..?
            log::debug!(
                "Fetched a pending block, got a closed one: block_number={:?} block_hash={:#x}",
                block.block_number,
                block.block_hash
            );
            return Ok(None);
        }
        MaybePendingBlockWithReceipts::PendingBlock(block) => block,
    };

    let class_update = fetch_class_updates(backend, &state_update, block_id, provider).await?;

    stopwatch_end!(sw, "fetching {:?}: {:?}", block_id);

    let (transactions, receipts) =
        block.transactions.into_iter().map(|t| (t.transaction.into(), t.receipt.into())).unzip();

    Ok(Some(UnverifiedPendingFullBlock {
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
    }))
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
    Ok((class_hash, contract_class))
}

#[cfg(test)]
mod test_l2_fetchers {
    use super::*;
    use crate::tests::utils::gateway::TestContext;
    use mc_block_import::UnverifiedPendingFullBlock;
    use mp_block::header::L1DataAvailabilityMode;
    use mp_chain_config::StarknetVersion;
    use starknet_api::felt;
    use starknet_providers::sequencer::models::BlockStatus;

    /// Test successful fetching of a pending block and updates.
    ///
    /// Verifies that:
    /// 1. The function correctly fetches a pending block.
    /// 2. The state update is properly retrieved.
    /// 3. Class updates are fetched and processed correctly.
    /// 4. The returned UnverifiedPendingFullBlock contains the expected data.
    #[tokio::test]
    async fn test_fetch_pending_block_and_updates_success() {
        let ctx = TestContext::new();

        // Mock the pending block
        ctx.mock_block_pending();

        // Mock class hash
        ctx.mock_class_hash("src/tests/utils/artifacts/class.json");

        let result = fetch_pending_block_and_updates(&ctx.backend, &ctx.provider).await;

        let pending_block = result
            .expect("Failed to fetch pending block")
            .expect("Expected Some(UnverifiedPendingFullBlock), got None");

        assert!(
            matches!(pending_block, UnverifiedPendingFullBlock { .. }),
            "Expected UnverifiedPendingFullBlock, got {:?}",
            pending_block
        );

        // Verify essential components of the pending block
        assert_eq!(
            pending_block.header.parent_block_hash,
            Some(felt!("0x1db054847816dbc0098c88915430c44da2c1e3f910fbcb454e14282baba0e75")),
            "Parent block hash should be same"
        );
        assert_eq!(
            pending_block.header.sequencer_address,
            felt!("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8"),
            "Sequencer address should be same"
        );
        assert!(pending_block.header.block_timestamp > 0, "Block timestamp should be greater than zero");
        assert_eq!(
            pending_block.header.protocol_version,
            StarknetVersion::new(0, 13, 2, 1),
            "Protocol version should match"
        );

        // Verify L1 gas prices
        assert_ne!(pending_block.header.l1_gas_price.eth_l1_gas_price, 0, "ETH L1 gas price should not be zero");
        assert_ne!(pending_block.header.l1_gas_price.strk_l1_gas_price, 0, "STRK L1 gas price should not be zero");
        assert_ne!(
            pending_block.header.l1_gas_price.eth_l1_data_gas_price, 0,
            "ETH L1 data gas price should not be zero"
        );
        assert_ne!(
            pending_block.header.l1_gas_price.strk_l1_data_gas_price, 0,
            "STRK L1 data gas price should not be zero"
        );

        // Verify L1 DA mode
        assert_eq!(pending_block.header.l1_da_mode, L1DataAvailabilityMode::Calldata, "L1 DA mode should be Calldata");

        // Verify state diff, transactions, and receipts
        assert!(
            !pending_block.state_diff.storage_diffs.is_empty()
                || !pending_block.state_diff.deployed_contracts.is_empty(),
            "State diff should not be empty"
        );
        assert!(pending_block.transactions.is_empty(), "Transactions should be empty");
        assert!(pending_block.receipts.is_empty(), "Receipts should be empty");
        assert_eq!(
            pending_block.transactions.len(),
            pending_block.receipts.len(),
            "Number of transactions should match number of receipts"
        );
    }

    /// Test error handling when fetching a pending block fails due to a provider error.
    ///
    /// Verifies that:
    /// 1. The function properly handles the case when a pending block is not found.
    /// 2. It returns an appropriate FetchError.
    #[tokio::test]
    async fn test_fetch_pending_block_and_updates_not_found() {
        let ctx = TestContext::new();

        // Mock a "pending block not found" scenario
        ctx.mock_block_pending_not_found();

        let result = fetch_pending_block_and_updates(&ctx.backend, &ctx.provider).await;

        assert!(
            matches!(result, Err(FetchError::Provider(ProviderError::StarknetError(StarknetError::BlockNotFound)))),
            "Expected BlockNotFound error, but got: {:?}",
            result
        );
    }

    /// Test successful fetching of state update with block for the pending block.
    ///
    /// Verifies that:
    /// 1. The function correctly fetches both state update and block for the pending block.
    /// 2. The returned data matches the expected format for a pending block.
    /// 3. Certain fields that should be None for pending blocks are indeed None.
    #[tokio::test]
    async fn test_fetch_state_update_with_block_pending() {
        let ctx = TestContext::new();

        // Mock the pending block
        ctx.mock_block_pending();

        let (state_update, block) = fetch_state_update_with_block(&ctx.provider, FetchBlockId::Pending)
            .await
            .expect("Failed to fetch state update with block");

        // Verify state update
        assert!(!state_update.state_diff.storage_diffs.is_empty(), "State update should contain storage diffs");
        assert!(
            !state_update.state_diff.deployed_contracts.is_empty(),
            "State update should contain deployed contracts"
        );

        // Verify block
        assert!(block.block_number.is_none(), "Pending block should not have a block number");
        assert!(block.block_hash.is_none(), "Pending block should not have a block hash");
        assert_eq!(
            block.parent_block_hash,
            felt!("0x1db054847816dbc0098c88915430c44da2c1e3f910fbcb454e14282baba0e75"),
            "Pending block should have a parent block hash"
        );
        assert!(block.state_root.is_none(), "Pending block should not have a state root");
        assert!(block.transactions.is_empty(), "Pending block should not contain transactions");
        assert_eq!(block.status, BlockStatus::Pending, "Pending block status should be 'PENDING'");
    }

    /// Test error handling when the requested block is not found.
    ///
    /// Verifies that:
    /// 1. The function returns a ProviderError::StarknetError(StarknetError::BlockNotFound) when the block doesn't exist.
    /// 2. The error is propagated correctly through the retry mechanism.
    #[tokio::test]
    async fn test_fetch_state_update_with_block_not_found() {
        let ctx = TestContext::new();

        // Mock a "block not found" scenario
        ctx.mock_block_not_found(5);

        let result = fetch_state_update_with_block(&ctx.provider, FetchBlockId::BlockN(5)).await;

        assert!(
            matches!(result, Err(ProviderError::StarknetError(StarknetError::BlockNotFound))),
            "Expected BlockNotFound error, but got: {:?}",
            result
        );
    }

    /// Test fetching with provider returning partial data.
    ///
    /// Verifies that:
    /// 1. The function correctly handles cases where the provider returns incomplete data.
    /// 2. It returns an appropriate error or retries as necessary.
    #[tokio::test]
    async fn test_fetch_state_update_with_block_partial_data() {
        let ctx = TestContext::new();

        // Mock partial data scenario
        ctx.mock_block_partial_data(5);
        ctx.mock_class_hash("src/tests/utils/artifacts/class.json");

        let result = fetch_state_update_with_block(&ctx.provider, FetchBlockId::BlockN(5)).await;

        assert!(
            matches!(
                result,
                Err(ProviderError::Other(ref e)) if e.to_string().contains("data did not match any variant of enum GatewayResponse")
            ),
            "Expected error about mismatched data, but got: {:?}",
            result
        );
    }

    /// Test fetching of class updates.
    ///
    /// This test ensures that:
    /// 1. Missing classes are correctly identified and fetched.
    /// 2. A known problematic class hash is properly handled.
    /// 3. The function returns the expected class update data.
    #[tokio::test]
    async fn test_fetch_class_updates() {
        let ctx = TestContext::new();

        ctx.mock_block(5);
        ctx.mock_class_hash("src/tests/utils/artifacts/class.json");

        let (state_update, _block) = fetch_state_update_with_block(&ctx.provider, FetchBlockId::BlockN(5))
            .await
            .expect("Failed to fetch state update with block");

        let class_updates = fetch_class_updates(&ctx.backend, &state_update, FetchBlockId::BlockN(5), &ctx.provider)
            .await
            .expect("Failed to fetch class updates");

        assert!(!class_updates.is_empty(), "Should have fetched at least one class update");

        // Verify the structure of the first class update
        let first_update = &class_updates[0];
        assert_ne!(first_update.class_hash(), Felt::ZERO, "Class hash should not be zero");
    }

    /// Test error handling in fetch_class_updates.
    ///
    /// Verifies that:
    /// 1. The function properly handles errors when checking for the classes that doesn't exist.
    /// 2. It handles errors during class fetching.
    #[tokio::test]
    async fn test_fetch_class_updates_error_handling() {
        let ctx = TestContext::new();

        ctx.mock_block(5);
        let (state_update, _block) =
            fetch_state_update_with_block(&ctx.provider, FetchBlockId::BlockN(5)).await.unwrap();
        ctx.mock_class_hash_not_found("0x78401746828463e2c3f92ebb261fc82f7d4d4c8d9a80a356c44580dab124cb0".to_string());
        let result = fetch_class_updates(&ctx.backend, &state_update, FetchBlockId::BlockN(5), &ctx.provider).await;

        assert!(
            matches!(
                result,
                Err(ref e) if matches!(
                    e.downcast_ref::<L2SyncError>(),
                    Some(L2SyncError::Provider(ProviderError::StarknetError(StarknetError::ClassHashNotFound)))
                )
            ),
            "Expected ClassHashNotFound error, but got: {:?}",
            result
        );
    }

    /// Test fetching of individual class definitions.
    ///
    /// Verifies that:
    /// 1. The function correctly fetches a class definition for a given hash.
    /// 2. It handles different block IDs correctly.
    /// 3. It returns the expected ContractClass structure.
    #[tokio::test]
    async fn test_fetch_class() {
        let ctx = TestContext::new();

        let class_hash = Felt::from_hex_unchecked("0x78401746828463e2c3f92ebb261fc82f7d4d4c8d9a80a356c44580dab124cb0");
        ctx.mock_class_hash("src/tests/utils/artifacts/class.json");

        let (fetched_hash, _contract_class) =
            fetch_class(class_hash, FetchBlockId::BlockN(5), &ctx.provider).await.expect("Failed to fetch class");

        assert_eq!(fetched_hash, class_hash, "Fetched class hash should match the requested one");
    }

    /// Test error handling in fetch_class.
    ///
    /// Verifies that:
    /// 1. The function properly handles provider errors.
    /// 2. It returns an appropriate ProviderError.
    #[tokio::test]
    async fn test_fetch_class_error_handling() {
        let ctx = TestContext::new();

        let class_hash = felt!("0x1234");
        ctx.mock_class_hash_not_found("0x1234".to_string());

        let result = fetch_class(class_hash, FetchBlockId::BlockN(5), &ctx.provider).await;

        assert!(
            matches!(result, Err(ProviderError::StarknetError(StarknetError::ClassHashNotFound))),
            "Expected ClassHashNotFound error, but got: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_fetch_state_update_works() {
        let ctx = TestContext::new();

        // Mock a block with a state update
        ctx.mock_block(5);

        let (state_update, block) = fetch_state_update_with_block(&ctx.provider, FetchBlockId::BlockN(5))
            .await
            .expect("Failed to fetch state update with block");

        // Verify state update
        assert!(!state_update.state_diff.storage_diffs.is_empty(), "State update should contain storage diffs");
        assert!(
            state_update.state_diff.deployed_contracts.is_empty(),
            "State update should not contain deployed contracts"
        );

        // Verify block
        assert_eq!(block.block_number, Some(5), "Block number should be 5");
        assert!(block.block_hash.is_some(), "Block should have a hash");
        assert!(block.parent_block_hash != Felt::ZERO, "Block should have a non-zero parent hash");
        assert!(block.state_root.is_some(), "Block should have a state root");
        assert_eq!(block.status, BlockStatus::AcceptedOnL1, "Block status should be AcceptedOnL1");

        // Verify some block details
        assert!(block.timestamp > 0, "Block timestamp should be greater than zero");
        assert!(block.transactions.is_empty(), "Block should not contain transactions");
        assert!(block.transaction_receipts.is_empty(), "Block should not contain transaction receipts");
    }
}
