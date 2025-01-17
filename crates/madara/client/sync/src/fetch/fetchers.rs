//! Contains the code required to fetch data from the network efficiently.
use super::FetchError;
use crate::l2::L2SyncError;
use anyhow::Context;
use core::time::Duration;
use futures::FutureExt;
use mc_block_import::{UnverifiedCommitments, UnverifiedFullBlock, UnverifiedPendingFullBlock};
use mc_gateway_client::GatewayProvider;
use mp_block::{BlockId, BlockTag};
use mp_class::class_update::{ClassUpdate, LegacyClassUpdate, SierraClassUpdate};
use mp_class::{ContractClass, MISSED_CLASS_HASHES};
use mp_gateway::block::{ProviderBlock, ProviderBlockPending};
use mp_gateway::error::{SequencerError, StarknetError, StarknetErrorCode};
use mp_gateway::state_update::ProviderStateUpdateWithBlockPendingMaybe::{self};
use mp_gateway::state_update::{ProviderStateUpdate, ProviderStateUpdatePending, StateDiff};
use mp_utils::service::MadaraServiceId;
use mp_utils::{stopwatch_end, PerfStopwatch};
use starknet_api::core::ChainId;
use starknet_types_core::felt::Felt;
use std::sync::Arc;
use url::Url;

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
    /// Whether to check the root of the state update.
    pub verify: bool,
    /// The optional API_KEY to avoid rate limiting from the sequencer gateway.
    pub api_key: Option<String>,
    /// Polling interval.
    pub sync_polling_interval: Option<Duration>,
    /// Number of blocks to sync (for testing purposes).
    pub n_blocks_to_sync: Option<u64>,
    /// Number of blocks between db flushes
    pub flush_every_n_blocks: u64,
    /// Number of seconds between db flushes
    pub flush_every_n_seconds: u64,
    /// Stops the node once all blocks have been synced (for testing purposes)
    pub stop_on_sync: bool,
    /// Number of blocks to fetch in parallel during the sync process
    pub sync_parallelism: u8,
    /// Warp update configuration
    pub warp_update: Option<WarpUpdateConfig>,
}

#[derive(Clone, Debug)]
pub struct WarpUpdateConfig {
    /// The port used for nodes to make rpc calls during a warp update.
    pub warp_update_port_rpc: u16,
    /// The port used for nodes to send blocks during a warp update.
    pub warp_update_port_fgw: u16,
    /// Whether to shutdown the warp update sender once the migration has completed.
    pub warp_update_shutdown_sender: bool,
    /// Whether to shut down the warp update receiver once the migration has completed
    pub warp_update_shutdown_receiver: bool,
    /// A list of services to start once warp update has completed.
    pub deferred_service_start: Vec<MadaraServiceId>,
    /// A list of services to stop one warp update has completed.
    pub deferred_service_stop: Vec<MadaraServiceId>,
}

pub async fn fetch_pending_block_and_updates(
    parent_block_hash: Felt,
    chain_id: &ChainId,
    provider: &GatewayProvider,
) -> Result<Option<UnverifiedPendingFullBlock>, FetchError> {
    let block_id = BlockId::Tag(BlockTag::Pending);
    let sw = PerfStopwatch::new();
    let block = retry(
        || async {
            match provider.get_state_update_with_block(block_id.clone()).await {
                Ok(block) => Ok(Some(block)),
                // Ignore (this is the case where we returned a closed block when we asked for a pending one)
                // When the FGW does not have a pending block, it can return the latest block instead
                Err(SequencerError::DeserializeBody { serde_error }) => {
                    tracing::debug!("Serde error when fetching the pending block: {serde_error:#}");
                    Ok(None)
                }
                Err(err) => Err(err),
            }
        },
        MAX_RETRY,
        BASE_DELAY,
    )
    .await?;

    let Some(block) = block else { return Ok(None) };

    let (state_update, block) = block.as_update_and_block();
    let (state_update, block) = (
        state_update.pending_owned().expect("Block should be pending (checked via serde above)"),
        block.pending_owned().expect("Block should be pending (checked via serde above)"),
    );

    if block.parent_block_hash != parent_block_hash {
        tracing::debug!(
            "Fetched a pending block, but mismatched parent block hash: parent_block_hash={:#x}",
            block.parent_block_hash
        );
        return Ok(None);
    }
    let class_update = fetch_class_updates(chain_id, &state_update.state_diff, block_id.clone(), provider).await?;

    stopwatch_end!(sw, "fetching {:?}: {:?}", block_id);

    let converted = convert_sequencer_block_pending(block, state_update, class_update)
        .context("Parsing the FGW pending block format")?;

    Ok(Some(converted))
}

pub async fn fetch_block_and_updates(
    chain_id: &ChainId,
    block_n: u64,
    provider: &GatewayProvider,
) -> Result<UnverifiedFullBlock, FetchError> {
    let block_id = BlockId::Number(block_n);

    let sw = PerfStopwatch::new();
    let (state_update, block) = retry(
        || async {
            provider
                .get_state_update_with_block(block_id.clone())
                .await
                .map(ProviderStateUpdateWithBlockPendingMaybe::as_update_and_block)
        },
        MAX_RETRY,
        BASE_DELAY,
    )
    .await?;
    let class_update = fetch_class_updates(chain_id, state_update.state_diff(), block_id, provider).await?;

    stopwatch_end!(sw, "fetching {:?}: {:?}", block_n);

    let converted = convert_sequencer_block_non_pending(
        block.non_pending_owned().expect("Block called on block number should not be pending"),
        state_update.non_pending_ownded().expect("State update called on block number should not be pending"),
        class_update,
    )
    .context("Parsing the FGW full block format")?;
    Ok(converted)
}

// TODO: should we be checking for cancellation here? This might take a while
async fn retry<F, Fut, T>(mut f: F, max_retries: u32, base_delay: Duration) -> Result<T, SequencerError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, SequencerError>>,
{
    let mut attempt = 0;
    loop {
        match f().await {
            Ok(res) => return Ok(res),
            Err(SequencerError::StarknetError(StarknetError { code: StarknetErrorCode::BlockNotFound, .. })) => {
                break Err(SequencerError::StarknetError(StarknetError::block_not_found()));
            }
            Err(err) => {
                let delay = base_delay * 2_u32.pow(attempt).min(6); // Cap to prevent overly long delays
                attempt += 1;
                if attempt > max_retries {
                    break Err(err);
                }

                if let SequencerError::StarknetError(StarknetError { code: StarknetErrorCode::RateLimited, .. }) = err {
                    tracing::info!("The fetching process has been rate limited, retrying in {:?}", delay)
                } else {
                    tracing::warn!("The provider has returned an error: {}, retrying in {:?}", err, delay)
                }

                tokio::time::sleep(delay).await;
            }
        }
    }
}

/// retrieves class updates from Starknet sequencer
async fn fetch_class_updates(
    chain_id: &ChainId,
    state_diff: &StateDiff,
    block_id: BlockId,
    provider: &GatewayProvider,
) -> anyhow::Result<Vec<ClassUpdate>> {
    // for blocks before 2597 on mainnet new classes are not declared in the state update
    // https://github.com/madara-alliance/madara/issues/233
    let legacy_classes: Vec<_> = match (chain_id, &block_id) {
        (ChainId::Mainnet, &BlockId::Number(block_n)) if block_n < 2597 => {
            MISSED_CLASS_HASHES.get(&block_n).cloned().unwrap_or_default()
        }
        _ => state_diff.old_declared_contracts.clone(),
    };

    let sierra_classes: Vec<_> = state_diff
        .declared_classes
        .iter()
        .map(|declared_class| (declared_class.class_hash, &declared_class.compiled_class_hash))
        .collect();

    let legacy_class_futures = legacy_classes.into_iter().map(|class_hash| {
        let block_id = block_id.clone();
        async move {
            let (class_hash, contract_class) =
                retry(|| fetch_class(class_hash, block_id.clone(), provider), MAX_RETRY, BASE_DELAY).await?;

            let ContractClass::Legacy(contract_class) = contract_class else {
                return Err(L2SyncError::UnexpectedClassType { class_hash });
            };
            let contract_class = Arc::try_unwrap(contract_class)
                .expect("Contract class should only have one referenced when it is fetched");

            Ok::<_, L2SyncError>(ClassUpdate::Legacy(LegacyClassUpdate { class_hash, contract_class }))
        }
        .boxed()
    });

    let sierra_class_futures = sierra_classes.into_iter().map(|(class_hash, &compiled_class_hash)| {
        let block_id = block_id.clone();
        async move {
            let (class_hash, contract_class) =
                retry(|| fetch_class(class_hash, block_id.clone(), provider), MAX_RETRY, BASE_DELAY).await?;

            let ContractClass::Sierra(contract_class) = contract_class else {
                return Err(L2SyncError::UnexpectedClassType { class_hash });
            };
            let contract_class = Arc::try_unwrap(contract_class)
                .expect("Contract class should only have one referenced when it is fetchd");

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
    block_id: BlockId,
    provider: &GatewayProvider,
) -> Result<(Felt, ContractClass), SequencerError> {
    let contract_class = provider.get_class_by_hash(class_hash, block_id).await?;
    tracing::debug!("Got the contract class {:?}", class_hash);
    Ok((class_hash, contract_class))
}

fn convert_sequencer_block_pending(
    block: ProviderBlockPending,
    state_update: ProviderStateUpdatePending,
    class_update: Vec<ClassUpdate>,
) -> anyhow::Result<UnverifiedPendingFullBlock> {
    Ok(UnverifiedPendingFullBlock {
        header: block.header()?,
        state_diff: state_update.state_diff.into(),
        receipts: block
            .transaction_receipts
            .into_iter()
            .zip(&block.transactions)
            .map(|(receipt, tx)| receipt.into_mp(tx))
            .collect(),
        transactions: block.transactions.into_iter().map(Into::into).collect(),
        declared_classes: class_update.into_iter().map(Into::into).collect(),
        ..Default::default()
    })
}

fn convert_sequencer_block_non_pending(
    block: ProviderBlock,
    state_update: ProviderStateUpdate,
    class_update: Vec<ClassUpdate>,
) -> anyhow::Result<UnverifiedFullBlock> {
    // Verify against these commitments.
    let commitments = UnverifiedCommitments {
        // TODO: these commitments are wrong for mainnet from block 0 to unknown. We need to figure out
        // which blocks and handle the case directly in the block import crate.
        // transaction_commitment: Some(block.transaction_commitment.context("No transaction commitment")?),
        // event_commitment: Some(block.event_commitment.context("No event commitment")?),
        state_diff_commitment: None,
        receipt_commitment: None,
        global_state_root: Some(block.state_root),
        block_hash: Some(block.block_hash),
        ..Default::default()
    };
    Ok(UnverifiedFullBlock {
        unverified_block_number: Some(block.block_number),
        header: block.header()?,
        state_diff: state_update.state_diff.into(),
        receipts: block
            .transaction_receipts
            .into_iter()
            .zip(&block.transactions)
            .map(|(receipt, tx)| receipt.into_mp(tx))
            .collect(),
        transactions: block.transactions.into_iter().map(Into::into).collect(),
        declared_classes: class_update.into_iter().map(Into::into).collect(),
        commitments,
        ..Default::default()
    })
}

#[cfg(test)]
#[path = "fetchers_real_fgw_test.rs"]
mod fetchers_real_fgw_test;

#[cfg(test)]
mod test_l2_fetchers {
    use super::*;
    use crate::tests::utils::gateway::{test_setup, TestContext};
    use mc_block_import::UnverifiedPendingFullBlock;
    use mc_db::MadaraBackend;
    use mp_block::header::L1DataAvailabilityMode;
    use mp_chain_config::StarknetVersion;
    use mp_gateway::block::BlockStatus;
    use rstest::*;
    use starknet_api::felt;
    use std::sync::Arc;

    /// Test successful fetching of a pending block and updates.
    ///
    /// Verifies that:
    /// 1. The function correctly fetches a pending block.
    /// 2. The state update is properly retrieved.
    /// 3. Class updates are fetched and processed correctly.
    /// 4. The returned UnverifiedPendingFullBlock contains the expected data.
    #[rstest]
    #[tokio::test]
    async fn test_fetch_pending_block_and_updates_success(test_setup: Arc<MadaraBackend>) {
        let ctx = TestContext::new(test_setup);

        // Mock the pending block
        ctx.mock_block_pending();

        // Mock class hash
        ctx.mock_class_hash(m_cairo_test_contracts::TEST_CONTRACT_SIERRA);

        let result = fetch_pending_block_and_updates(
            Felt::from_hex_unchecked("0x1db054847816dbc0098c88915430c44da2c1e3f910fbcb454e14282baba0e75"),
            &ctx.backend.chain_config().chain_id,
            &ctx.provider,
        )
        .await;

        let pending_block = result.expect("Failed to fetch pending block").expect("No pending block");

        assert!(
            matches!(pending_block, UnverifiedPendingFullBlock { .. }),
            "Expected UnverifiedPendingFullBlock, got {:?}",
            pending_block
        );

        // Verify essential components of the pending block
        assert_eq!(
            pending_block.header.parent_block_hash,
            Some(felt!("0x1db054847816dbc0098c88915430c44da2c1e3f910fbcb454e14282baba0e75")),
            "Parent block hash should match"
        );
        assert_eq!(
            pending_block.header.sequencer_address,
            felt!("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8"),
            "Sequencer address should match"
        );
        assert_eq!(pending_block.header.block_timestamp.0, 1725950824, "Block timestamp should match");
        assert_eq!(
            pending_block.header.protocol_version,
            StarknetVersion::new(0, 13, 2, 1),
            "Protocol version should match"
        );

        // Verify L1 gas prices
        assert_eq!(pending_block.header.l1_gas_price.eth_l1_gas_price, 0x274287586, "ETH L1 gas price should match");
        assert_eq!(
            pending_block.header.l1_gas_price.strk_l1_gas_price, 0x363cc34e29f8,
            "STRK L1 gas price should match"
        );
        assert_eq!(
            pending_block.header.l1_gas_price.eth_l1_data_gas_price, 0x2bc1e42413,
            "ETH L1 data gas price should match"
        );
        assert_eq!(
            pending_block.header.l1_gas_price.strk_l1_data_gas_price, 0x3c735d85586c2,
            "STRK L1 data gas price should match"
        );

        // Verify L1 DA mode
        assert_eq!(pending_block.header.l1_da_mode, L1DataAvailabilityMode::Calldata, "L1 DA mode should be Calldata");

        // Verify state diff
        assert!(!pending_block.state_diff.storage_diffs.is_empty(), "Storage diffs should not be empty");
        assert_eq!(pending_block.state_diff.storage_diffs.len(), 1, "Should have 1 storage diff");
        assert_eq!(
            pending_block.state_diff.storage_diffs[0].address,
            felt!("0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d"),
            "Storage diff key should match"
        );
        assert_eq!(pending_block.state_diff.deployed_contracts.len(), 2, "Should have 2 deployed contracts");
        assert_eq!(pending_block.state_diff.nonces.len(), 2, "Should have 2 nonces");

        // Verify transactions and receipts
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
    #[rstest]
    #[tokio::test]
    async fn test_fetch_pending_block_and_updates_not_found(test_setup: Arc<MadaraBackend>) {
        let ctx = TestContext::new(test_setup);

        // Mock a "pending block not found" scenario
        ctx.mock_block_pending_not_found();

        let result = fetch_pending_block_and_updates(
            Felt::from_hex_unchecked("0x1db054847816dbc0098c88915430c44da2c1e3f910fbcb454e14282baba0e75"),
            &ctx.backend.chain_config().chain_id,
            &ctx.provider,
        )
        .await;

        assert!(
            matches!(
                result,
                Err(FetchError::Sequencer(SequencerError::StarknetError(StarknetError {
                    code: StarknetErrorCode::BlockNotFound,
                    ..
                })))
            ),
            "Expected no block, but got: {:?}",
            result
        );
    }

    /// Test successful fetching of state update with block for the pending block.
    ///
    /// Verifies that:
    /// 1. The function correctly fetches both state update and block for the pending block.
    /// 2. The returned data matches the expected format for a pending block.
    /// 3. Certain fields that should be None for pending blocks are indeed None.
    #[rstest]
    #[tokio::test]
    async fn test_fetch_state_update_with_block_pending(test_setup: Arc<MadaraBackend>) {
        let ctx = TestContext::new(test_setup);

        // Mock the pending block
        ctx.mock_block_pending();

        let (state_update, block) = ctx
            .provider
            .get_state_update_with_block(BlockId::Tag(BlockTag::Pending))
            .await
            .expect("Failed to fetch state update with block on tag pending")
            .as_update_and_block();
        let (state_update, block) = (
            state_update.pending().expect("State update called on tag 'pending' should be pending"),
            block.pending().expect("Block called on tag 'pending' should be pending"),
        );

        // Verify state update
        assert_eq!(
            state_update.old_root,
            felt!("0x37817010d31db557217addb3b4357c2422c8d8de0290c3f6a867bbdc49c32a0"),
            "Old root should match"
        );

        // Verify storage diffs
        assert_eq!(state_update.state_diff.storage_diffs.len(), 1, "Should have 1 storage diff");
        let storage_diff = state_update
            .state_diff
            .storage_diffs
            .get(&felt!("0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d"))
            .expect("Storage diff should exist");
        assert_eq!(storage_diff.len(), 2, "Should have 2 storage entries");
        assert_eq!(
            storage_diff[0].key,
            felt!("0x5496768776e3db30053404f18067d81a6e06f5a2b0de326e21298fd9d569a9a"),
            "First storage key should match"
        );
        assert_eq!(storage_diff[0].value, felt!("0x1b7622454b6cea6e76bb2"), "First storage value should match");
        assert_eq!(
            storage_diff[1].key,
            felt!("0x5928e5598505749c60b49cc98e3acd5f3faa4a36910f50824395385b3c3a5c6"),
            "Second storage key should match"
        );
        assert_eq!(storage_diff[1].value, felt!("0xdefb9937f1c6af5096"), "Second storage value should match");

        // Verify nonces
        assert_eq!(state_update.state_diff.nonces.len(), 2, "Should have 2 nonces");
        assert_eq!(
            state_update
                .state_diff
                .nonces
                .get(&felt!("0x596d7421536f9d895015f207a6a349f54081634a25d4b403d3cd0363208ee1c"))
                .unwrap(),
            &felt!("0x2"),
            "First nonce should match"
        );
        assert_eq!(
            state_update
                .state_diff
                .nonces
                .get(&felt!("0x2bb8a1f5a1241c1ebe8e10ff93b38ab097b1a20f77517997f8799829e096535"))
                .unwrap(),
            &felt!("0x18ab"),
            "Second nonce should match"
        );

        // Verify deployed contracts
        assert_eq!(state_update.state_diff.deployed_contracts.len(), 2, "Should have 2 deployed contracts");
        assert_eq!(
            state_update.state_diff.deployed_contracts[0].address,
            felt!("0x596d7421536f9d895015f207a6a349f54081634a25d4b403d3cd0363208ee1c"),
            "First deployed contract address should match"
        );
        assert_eq!(
            state_update.state_diff.deployed_contracts[0].class_hash,
            felt!("0x36078334509b514626504edc9fb252328d1a240e4e948bef8d0c08dff45927f"),
            "First deployed contract class hash should match"
        );
        assert_eq!(
            state_update.state_diff.deployed_contracts[1].address,
            felt!("0x7ab19cc28b12535df410edd1dbaad521ee83479b5936e00decdde5dd566c8b7"),
            "Second deployed contract address should match"
        );
        assert_eq!(
            state_update.state_diff.deployed_contracts[1].class_hash,
            felt!("0x4ccf6144da19dc18c9f109a8a46e66ea2e08b2f22b03f895a715968d26622ea"),
            "Second deployed contract class hash should match"
        );

        assert!(state_update.state_diff.old_declared_contracts.is_empty(), "Should have no old declared contracts");
        assert!(state_update.state_diff.declared_classes.is_empty(), "Should have no declared classes");
        assert!(state_update.state_diff.replaced_classes.is_empty(), "Should have no replaced classes");

        // Verify block
        assert!(block.transactions.is_empty(), "Pending block should not contain transactions");
        assert_eq!(block.status, BlockStatus::Pending, "Pending block status should be 'PENDING'");
        assert_eq!(block.l1_da_mode, L1DataAvailabilityMode::Calldata, "L1 DA mode should be CALLDATA");
        assert_eq!(block.l1_gas_price.price_in_wei, 10538743174, "L1 gas price in wei should match");
        assert_eq!(block.l1_gas_price.price_in_fri, 59634602617336, "L1 gas price in fri should match");
        assert_eq!(block.l1_data_gas_price.price_in_wei, 187936547859, "L1 data gas price in wei should match");
        assert_eq!(block.l1_data_gas_price.price_in_fri, 1063459006809794, "L1 data gas price in fri should match");
        assert_eq!(block.timestamp, 1725950824, "Timestamp should match");
        assert_eq!(
            block.sequencer_address,
            felt!("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8"),
            "Sequencer address should match"
        );
        assert!(block.transaction_receipts.is_empty(), "Should have no transaction receipts");
        assert_eq!(block.starknet_version, Some("0.13.2.1".to_string()), "Starknet version should match");
    }

    /// Test error handling when the requested block is not found.
    ///
    /// Verifies that:
    /// 1. The function returns a ProviderError::StarknetError(StarknetError::BlockNotFound) when the block doesn't exist.
    /// 2. The error is propagated correctly through the retry mechanism.
    #[rstest]
    #[tokio::test]
    async fn test_fetch_state_update_with_block_not_found(test_setup: Arc<MadaraBackend>) {
        let ctx = TestContext::new(test_setup);

        // Mock a "block not found" scenario
        ctx.mock_block_not_found(5);

        let result = ctx.provider.get_state_update_with_block(BlockId::Number(5)).await;

        assert!(
            matches!(
                result,
                Err(SequencerError::StarknetError(StarknetError { code: StarknetErrorCode::BlockNotFound, .. }))
            ),
            "Expected BlockNotFound error, but got: {:?}",
            result
        );
    }

    /// Test fetching with provider returning partial data.
    ///
    /// Verifies that:
    /// 1. The function correctly handles cases where the provider returns incomplete data.
    /// 2. It returns an appropriate error or retries as necessary.
    #[rstest]
    #[tokio::test]
    async fn test_fetch_state_update_with_block_partial_data(test_setup: Arc<MadaraBackend>) {
        let ctx = TestContext::new(test_setup);

        // Mock partial data scenario
        ctx.mock_block_partial_data(5);
        ctx.mock_class_hash(m_cairo_test_contracts::TEST_CONTRACT_SIERRA);

        let result = ctx.provider.get_state_update_with_block(BlockId::Number(5)).await;

        assert!(
            matches!(result, Err(SequencerError::DeserializeBody { .. })),
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
    #[rstest]
    #[tokio::test]
    async fn test_fetch_class_updates(test_setup: Arc<MadaraBackend>) {
        let ctx = TestContext::new(test_setup);

        ctx.mock_block(5);
        ctx.mock_class_hash(m_cairo_test_contracts::TEST_CONTRACT_SIERRA);

        // WARN: the mock server is set up to ALWAYS return state update with
        // block, DO NOT call `get_state_update` on it!
        let state_update = ctx
            .provider
            .get_state_update_with_block(BlockId::Number(5))
            .await
            .expect("Failed to fetch state update at block number 5")
            .state_update();
        let state_diff = state_update.state_diff();

        let class_updates =
            fetch_class_updates(&ctx.backend.chain_config().chain_id, state_diff, BlockId::Number(5), &ctx.provider)
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
    #[rstest]
    #[tokio::test]
    async fn test_fetch_class_updates_error_handling(test_setup: Arc<MadaraBackend>) {
        let ctx = TestContext::new(test_setup);

        ctx.mock_block(5);
        let state_update = ctx
            .provider
            .get_state_update_with_block(BlockId::Number(5))
            .await
            .expect("Failed to fetch state update at block number 5")
            .state_update();
        let state_diff = state_update.state_diff();

        ctx.mock_class_hash_not_found("0x40fe2533528521fc49a8ad8440f8a1780c50337a94d0fce43756015fa816a8a".to_string());
        let result =
            fetch_class_updates(&ctx.backend.chain_config().chain_id, state_diff, BlockId::Number(5), &ctx.provider)
                .await;

        assert!(matches!(
        result,
        Err(ref e) if matches!(
            e.downcast_ref::<L2SyncError>(),
            Some(L2SyncError::SequencerError(SequencerError::StarknetError(StarknetError {code: StarknetErrorCode::UndeclaredClass, ..}))))
        ));
    }

    /// Test fetching of individual class definitions.
    ///
    /// Verifies that:
    /// 1. The function correctly fetches a class definition for a given hash.
    /// 2. It handles different block IDs correctly.
    /// 3. It returns the expected ContractClass structure.
    #[rstest]
    #[tokio::test]
    async fn test_fetch_class(test_setup: Arc<MadaraBackend>) {
        let ctx = TestContext::new(test_setup);

        let class_hash = Felt::from_hex_unchecked("0x78401746828463e2c3f92ebb261fc82f7d4d4c8d9a80a356c44580dab124cb0");
        ctx.mock_class_hash(m_cairo_test_contracts::TEST_CONTRACT_SIERRA);

        let (fetched_hash, _contract_class) =
            fetch_class(class_hash, BlockId::Number(5), &ctx.provider).await.expect("Failed to fetch class");

        assert_eq!(fetched_hash, class_hash, "Fetched class hash should match the requested one");
    }

    /// Test error handling in fetch_class.
    ///
    /// Verifies that:
    /// 1. The function properly handles provider errors.
    /// 2. It returns an appropriate ProviderError.
    #[rstest]
    #[tokio::test]
    async fn test_fetch_class_error_handling(test_setup: Arc<MadaraBackend>) {
        let ctx = TestContext::new(test_setup);

        let class_hash = felt!("0x1234");
        ctx.mock_class_hash_not_found("0x1234".to_string());

        let result = fetch_class(class_hash, BlockId::Number(5), &ctx.provider).await;

        assert!(
            matches!(
                result,
                Err(SequencerError::StarknetError(StarknetError { code: StarknetErrorCode::UndeclaredClass, .. }))
            ),
            "Expected ClassHashNotFound error, but got: {:?}",
            result
        );
    }

    #[rstest]
    #[tokio::test]
    async fn test_fetch_state_update_works(test_setup: Arc<MadaraBackend>) {
        let ctx = TestContext::new(test_setup);

        // Mock a block with a state update
        ctx.mock_block(5);

        let (state_update, block) = ctx
            .provider
            .get_state_update_with_block(BlockId::Number(5))
            .await
            .expect("Failed to fetch state update with block at block number 5")
            .as_update_and_block();
        let (state_update, block) = (
            state_update.non_pending().expect("State update at block number 5 should not be pending"),
            block.non_pending().expect("Block at block number 5 should not be pending"),
        );

        // Verify state update
        assert_eq!(state_update.block_hash, felt!("0x541112d5d5937a66ff09425a0256e53ac5c4f554be7e24917fc21a71aa3cf32"));
        assert_eq!(state_update.new_root, felt!("0x704b7fe29fa070cf3737173acd1d0790fe318f68cc07a49ddfa9c1cd94c804f"));
        assert_eq!(state_update.old_root, felt!("0x6152bda357cb522337756c71bcab298d88c5d829a479ad8247b82b969912713"));

        // Verify storage diffs
        assert_eq!(state_update.state_diff.storage_diffs.len(), 6);
        let storage_diff = state_update
            .state_diff
            .storage_diffs
            .get(&felt!("0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d"))
            .unwrap();
        assert_eq!(storage_diff.len(), 2);
        assert_eq!(storage_diff[0].key, felt!("0x5496768776e3db30053404f18067d81a6e06f5a2b0de326e21298fd9d569a9a"));
        assert_eq!(storage_diff[0].value, felt!("0x1b77017df88b0858c9c29"));

        // Verify nonces
        assert_eq!(state_update.state_diff.nonces.len(), 2);
        assert_eq!(
            state_update
                .state_diff
                .nonces
                .get(&felt!("0x5005f66205d5d1c08d23b2046a9fa44f27a21dc1ea205bd33c5d7c667df2d7b"))
                .unwrap(),
            &felt!("0x33f0")
        );

        // Verify other state diff components
        assert!(state_update.state_diff.deployed_contracts.is_empty());
        assert!(state_update.state_diff.old_declared_contracts.is_empty());
        assert_eq!(state_update.state_diff.declared_classes.len(), 1);
        assert_eq!(
            state_update.state_diff.declared_classes[0].class_hash,
            felt!("0x40fe2533528521fc49a8ad8440f8a1780c50337a94d0fce43756015fa816a8a")
        );
        assert!(state_update.state_diff.replaced_classes.is_empty());

        // Verify block
        assert_eq!(block.block_number, 5);
        assert_eq!(block.block_hash, felt!("0x541112d5d5937a66ff09425a0256e53ac5c4f554be7e24917fc21a71aa3cf32"));
        assert_eq!(block.parent_block_hash, felt!("0x6dc4eb6311529b941e3963f477b1d13928b38dd4c6ec0206bfba73c8a87198d"));
        assert_eq!(block.state_root, felt!("0x704b7fe29fa070cf3737173acd1d0790fe318f68cc07a49ddfa9c1cd94c804f"));
        assert_eq!(block.status, BlockStatus::AcceptedOnL1);
        assert_eq!(block.l1_da_mode, L1DataAvailabilityMode::Calldata);

        // Verify gas prices
        assert_eq!(block.l1_gas_price.price_in_wei, 16090604261);
        assert_eq!(block.l1_gas_price.price_in_fri, 94420157521538);
        assert_eq!(block.l1_data_gas_price.price_in_wei, 273267212519);
        assert_eq!(block.l1_data_gas_price.price_in_fri, 1603540353922810);

        assert_eq!(block.timestamp, 1725974819);
        assert_eq!(
            block.sequencer_address,
            Some(felt!("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8"))
        );
        assert!(block.transactions.is_empty());
        assert!(block.transaction_receipts.is_empty());
        assert_eq!(block.starknet_version, Some("0.13.2.1".to_string()));
    }
}
