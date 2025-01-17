use std::sync::Arc;

use crate::client::{L1BlockMetrics, StarknetCoreContract};
use crate::{client::EthereumClient, utils::convert_log_state_update};
use anyhow::Context;
use futures::StreamExt;
use mc_db::MadaraBackend;
use mp_utils::service::ServiceContext;
use mp_utils::trim_hash;
use serde::Deserialize;
use starknet_types_core::felt::Felt;

const ERR_ARCHIVE: &str =
    "Failed to watch event filter - Ensure you are using an L1 RPC endpoint that points to an archive node";

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

pub fn update_l1(
    backend: &MadaraBackend,
    state_update: L1StateUpdate,
    block_metrics: &L1BlockMetrics,
) -> anyhow::Result<()> {
    tracing::info!(
        "🔄 Updated L1 head #{} ({}) with state root ({})",
        state_update.block_number,
        trim_hash(&state_update.block_hash),
        trim_hash(&state_update.global_root)
    );

    block_metrics.l1_block_number.record(state_update.block_number, &[]);

    backend.write_last_confirmed_block(state_update.block_number).context("Setting l1 last confirmed block number")?;
    tracing::debug!("update_l1: wrote last confirmed block number");

    Ok(())
}

pub async fn state_update_worker(
    backend: Arc<MadaraBackend>,
    eth_client: Arc<EthereumClient>,
    mut ctx: ServiceContext,
) -> anyhow::Result<()> {
    // Clear L1 confirmed block at startup
    backend.clear_last_confirmed_block().context("Clearing l1 last confirmed block number")?;
    tracing::debug!("update_l1: cleared confirmed block number");

    tracing::info!("🚀 Subscribed to L1 state verification");
    // This does not seem to play well with anvil
    #[cfg(not(test))]
    {
        let initial_state = get_initial_state(&eth_client).await.context("Getting initial ethereum state")?;
        update_l1(&backend, initial_state, &eth_client.l1_block_metrics)?;
    }

    // Listen to LogStateUpdate (0x77552641) update and send changes continuously
    let event_filter = eth_client.l1_core_contract.event_filter::<StarknetCoreContract::LogStateUpdate>();

    let mut event_stream = match ctx.run_until_cancelled(event_filter.watch()).await {
        Some(res) => res.context(ERR_ARCHIVE)?.into_stream(),
        None => return anyhow::Ok(()),
    };

    while let Some(Some(event_result)) = ctx.run_until_cancelled(event_stream.next()).await {
        let log = event_result.context("listening for events")?;
        let format_event: L1StateUpdate =
            convert_log_state_update(log.0.clone()).context("formatting event into an L1StateUpdate")?;
        update_l1(&backend, format_event, &eth_client.l1_block_metrics)?;
    }

    anyhow::Ok(())
}

#[cfg(test)]
mod eth_client_event_subscription_test {
    use super::*;
    use std::{sync::Arc, time::Duration};

    use alloy::{node_bindings::Anvil, providers::ProviderBuilder, sol};
    use mc_db::DatabaseService;
    use mp_chain_config::ChainConfig;
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
        let chain_info = Arc::new(ChainConfig::madara_test());

        // Set up database paths
        let temp_dir = TempDir::new().expect("issue while creating temporary directory");
        let base_path = temp_dir.path().join("data");
        let backup_dir = Some(temp_dir.path().join("backups"));

        // Initialize database service
        let db = Arc::new(
            DatabaseService::new(&base_path, backup_dir, false, chain_info.clone(), Default::default())
                .await
                .expect("Failed to create database service"),
        );

        // Set up metrics service
        let l1_block_metrics = L1BlockMetrics::register().unwrap();

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
                state_update_worker(Arc::clone(db.backend()), Arc::new(eth_client), ServiceContext::new_for_testing())
                    .await
                    .unwrap()
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
