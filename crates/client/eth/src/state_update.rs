use crate::client::{L1BlockMetrics, StarknetCoreContract};
use crate::{
    client::EthereumClient,
    utils::{convert_log_state_update, trim_hash},
};
use anyhow::Context;
use futures::StreamExt;
use mc_db::MadaraBackend;
use mp_transactions::MAIN_CHAIN_ID;
use mp_utils::channel_wait_or_graceful_shutdown;
use serde::Deserialize;
use starknet_types_core::felt::Felt;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct L1StateUpdate {
    pub block_number: u64,
    pub global_root: Felt,
    pub block_hash: Felt,
}

/// Get the last Starknet state update verified on the L1
pub async fn get_initial_state(client: &EthereumClient) -> anyhow::Result<L1StateUpdate> {
    let block_number = client.get_last_verified_block_number().await?;
    let block_hash = client.get_last_verified_block_hash().await?;
    let global_root = client.get_last_state_root().await?;

    Ok(L1StateUpdate { global_root, block_number, block_hash })
}

/// Subscribes to the LogStateUpdate event from the Starknet core contract and store latest
/// verified state
pub async fn listen_and_update_state(
    eth_client: &EthereumClient,
    backend: &MadaraBackend,
    block_metrics: &L1BlockMetrics,
    chain_id: Felt,
) -> anyhow::Result<()> {
    let event_filter = eth_client.l1_core_contract.event_filter::<StarknetCoreContract::LogStateUpdate>();

    let mut event_stream = event_filter.watch().await.context("Failed to watch event filter")?.into_stream();

    while let Some(event_result) = channel_wait_or_graceful_shutdown(event_stream.next()).await {
        let log = event_result.context("listening for events")?;
        let format_event: L1StateUpdate =
            convert_log_state_update(log.0.clone()).context("formatting event into an L1StateUpdate")?;
        update_l1(backend, format_event, block_metrics, chain_id)?;
    }

    Ok(())
}

pub fn update_l1(
    backend: &MadaraBackend,
    state_update: L1StateUpdate,
    block_metrics: &L1BlockMetrics,
    chain_id: Felt,
) -> anyhow::Result<()> {
    // This is a provisory check to avoid updating the state with an L1StateUpdate that should not have been detected
    //
    // TODO: Remove this check when the L1StateUpdate is properly verified
    if state_update.block_number > 500000u64 || chain_id == MAIN_CHAIN_ID {
        log::info!(
            "🔄 Updated L1 head #{} ({}) with state root ({})",
            state_update.block_number,
            trim_hash(&state_update.block_hash),
            trim_hash(&state_update.global_root)
        );

        block_metrics.l1_block_number.set(state_update.block_number as f64);

        backend
            .write_last_confirmed_block(state_update.block_number)
            .context("Setting l1 last confirmed block number")?;
        log::debug!("update_l1: wrote last confirmed block number");
    }

    Ok(())
}

pub async fn state_update_worker(
    backend: &MadaraBackend,
    eth_client: &EthereumClient,
    chain_id: Felt,
) -> anyhow::Result<()> {
    // Clear L1 confirmed block at startup
    backend.clear_last_confirmed_block().context("Clearing l1 last confirmed block number")?;
    log::debug!("update_l1: cleared confirmed block number");

    log::info!("🚀 Subscribed to L1 state verification");
    // ideally here there would be one service which will update the l1 gas prices and another one for messages and one that's already present is state update
    // Get and store the latest verified state
    let initial_state = get_initial_state(eth_client).await.context("Getting initial ethereum state")?;
    update_l1(backend, initial_state, &eth_client.l1_block_metrics, chain_id)?;

    // Listen to LogStateUpdate (0x77552641) update and send changes continusly
    listen_and_update_state(eth_client, backend, &eth_client.l1_block_metrics, chain_id)
        .await
        .context("Subscribing to the LogStateUpdate event")?;

    Ok(())
}

#[cfg(test)]
mod eth_client_event_subscription_test {
    use super::*;
    use std::{sync::Arc, time::Duration};

    use alloy::{node_bindings::Anvil, providers::ProviderBuilder, sol};
    use mc_db::DatabaseService;
    use mc_metrics::{MetricsRegistry, MetricsService};
    use mp_chain_config::ChainConfig;
    use mp_convert::ToFelt;
    use rstest::*;
    use tempfile::TempDir;
    use url::Url;

    sol!(
        #[sol(rpc, bytecode="6080604052348015600e575f80fd5b506101618061001c5f395ff3fe608060405234801561000f575f80fd5b5060043610610029575f3560e01c80634185df151461002d575b5f80fd5b610035610037565b005b5f7f0639349b21e886487cd6b341de2050db8ab202d9c6b0e7a2666d598e5fcf81a690505f620a1caf90505f7f0279b69383ea92624c1ae4378ac7fae6428f47bbd21047ea0290c3653064188590507fd342ddf7a308dec111745b00315c14b7efb2bdae570a6856e088ed0c65a3576c8383836040516100b9939291906100f6565b60405180910390a1505050565b5f819050919050565b6100d8816100c6565b82525050565b5f819050919050565b6100f0816100de565b82525050565b5f6060820190506101095f8301866100cf565b61011660208301856100e7565b61012360408301846100cf565b94935050505056fea2646970667358221220fbc6fd165c86ed9af0c5fcab2830d4a72894fd6a98e9c16dbf9101c4c22e2f7d64736f6c634300081a0033")]
        contract DummyContract {
            event LogStateUpdate(uint256 globalRoot, int256 blockNumber, uint256 blockHash);

            function fireEvent() public {
                uint256 globalRoot = 2814950447364693428789615812443623689251959344851195711990387747563915674022;
                int256 blockNumber = 662703;
                uint256 blockHash = 1119674286844400689540394420005977072742999649767515920196535047615668295813;

                emit LogStateUpdate(globalRoot, blockNumber, blockHash);
            }
        }
    );

    const L2_BLOCK_NUMBER: u64 = 662703;
    const ANOTHER_ANVIL_PORT: u16 = 8548;
    const EVENT_PROCESSING_TIME: u64 = 2; // Time to allow for event processing in seconds

    /// Test the event subscription and state update functionality
    ///
    /// This test performs the following steps:
    /// 1. Sets up a mock Ethereum environment using Anvil
    /// 2. Initializes necessary services (Database, Metrics)
    /// 3. Deploys a dummy contract and sets up an Ethereum client
    /// 4. Starts listening for state updates
    /// 5. Fires an event from the dummy contract
    /// 6. Waits for event processing and verifies the block number
    #[rstest]
    #[tokio::test]
    async fn listen_and_update_state_when_event_fired_works() {
        // Start Anvil instance
        let anvil = Anvil::new()
            .block_time(1)
            .chain_id(1337)
            .port(ANOTHER_ANVIL_PORT)
            .try_spawn()
            .expect("failed to spawn anvil instance");
        println!("Anvil started and running at `{}`", anvil.endpoint());

        // Set up chain info
        let chain_info = Arc::new(ChainConfig::test_config().unwrap());

        // Set up database paths
        let temp_dir = TempDir::new().expect("issue while creating temporary directory");
        let base_path = temp_dir.path().join("data");
        let backup_dir = Some(temp_dir.path().join("backups"));

        // Initialize database service
        let db = Arc::new(
            DatabaseService::new(&base_path, backup_dir, false, chain_info.clone(), &MetricsRegistry::dummy())
                .await
                .expect("Failed to create database service"),
        );

        // Set up metrics service
        let prometheus_service = MetricsService::new(true, false, 9615).unwrap();
        let l1_block_metrics = L1BlockMetrics::register(prometheus_service.registry()).unwrap();

        let rpc_url: Url = anvil.endpoint().parse().expect("issue while parsing");
        let provider = ProviderBuilder::new().on_http(rpc_url);

        let contract = DummyContract::deploy(provider.clone()).await.unwrap();
        let core_contract = StarknetCoreContract::new(*contract.address(), provider.clone());

        let eth_client =
            EthereumClient { provider: Arc::new(provider), l1_core_contract: core_contract.clone(), l1_block_metrics };

        // Start listening for state updates
        let listen_handle = {
            let db = Arc::clone(&db);
            tokio::spawn(async move {
                listen_and_update_state(
                    &eth_client,
                    db.backend(),
                    &eth_client.l1_block_metrics,
                    chain_info.chain_id.clone().to_felt(),
                )
                .await
            })
        };

        let _ = contract.fireEvent().send().await.expect("Failed to fire event");

        // Wait for event processing
        tokio::time::sleep(Duration::from_secs(EVENT_PROCESSING_TIME)).await;

        // Verify the block number
        let block_in_db =
            db.backend().get_l1_last_confirmed_block().expect("Failed to get L1 last confirmed block number");

        // Explicitly cancel the listen task, else it would be running in the background
        listen_handle.abort();
        assert_eq!(block_in_db, Some(L2_BLOCK_NUMBER), "Block in DB does not match expected L2 block number");
    }
}
