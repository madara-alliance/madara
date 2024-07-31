use alloy::primitives::Address;
use anyhow::Context;
use dc_db::DeoxysBackend;
use dc_metrics::block_metrics::block_metrics::BlockMetrics;
use dp_convert::ToFelt;
use dp_transactions::TEST_CHAIN_ID;
use dp_utils::channel_wait_or_graceful_shutdown;
use futures::StreamExt;
use serde::Deserialize;
use starknet_api::hash::StarkHash;
use starknet_types_core::felt::Felt;
use url::Url;

use crate::{
    client::{EthereumClient, StarknetCoreContract},
    utils::{convert_log_state_update, trim_hash},
};

#[derive(Debug, Clone, Deserialize)]
pub struct L1StateUpdate {
    pub block_number: u64,
    pub global_root: StarkHash,
    pub block_hash: StarkHash,
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
    eth_client: EthereumClient,
    backend: &DeoxysBackend,
    block_metrics: BlockMetrics,
    chain_id: Felt,
) -> anyhow::Result<()> {
    let event_filter = eth_client.l1_core_contract.event_filter::<StarknetCoreContract::LogStateUpdate>();

    let mut event_stream = event_filter.watch().await.context("Failed to watch event filter")?.into_stream();

    while let Some(event_result) = channel_wait_or_graceful_shutdown(event_stream.next()).await {
        let log = event_result.context("listening for events")?;
        let format_event: L1StateUpdate =
            convert_log_state_update(log.0.clone()).context("formatting event into an L1StateUpdate")?;
        update_l1(backend, format_event, block_metrics.clone(), chain_id)?;
    }

    Ok(())
}

pub fn update_l1(
    backend: &DeoxysBackend,
    state_update: L1StateUpdate,
    block_metrics: BlockMetrics,
    chain_id: Felt,
) -> anyhow::Result<()> {
    // This is a provisory check to avoid updating the state with an L1StateUpdate that should not have been detected
    //
    // TODO: Remove this check when the L1StateUpdate is properly verified
    if state_update.block_number > 500000u64 || chain_id == TEST_CHAIN_ID {
        log::info!(
            "🔄 Updated L1 head #{} ({}) with state root ({})",
            state_update.block_number,
            trim_hash(&state_update.block_hash.to_felt()),
            trim_hash(&state_update.global_root.to_felt())
        );

        block_metrics.l1_block_number.set(state_update.block_number as f64);

        backend
            .write_last_confirmed_block(state_update.block_number)
            .context("Setting l1 last confirmed block number")?;
        log::debug!("update_l1: wrote last confirmed block number");
    }

    Ok(())
}

pub async fn sync(
    backend: &DeoxysBackend,
    l1_url: Url,
    block_metrics: BlockMetrics,
    l1_core_address: Address,
    chain_id: Felt,
) -> anyhow::Result<()> {
    // Clear L1 confirmed block at startup
    backend.clear_last_confirmed_block().context("Clearing l1 last confirmed block number")?;
    log::debug!("update_l1: cleared confirmed block number");

    let client = EthereumClient::new(l1_url, l1_core_address).await.context("Creating ethereum client")?;

    log::info!("🚀 Subscribed to L1 state verification");

    // Get and store the latest verified state
    let initial_state = get_initial_state(&client).await.context("Getting initial ethereum state")?;
    update_l1(backend, initial_state, block_metrics.clone(), chain_id)?;

    // Listen to LogStateUpdate (0x77552641) update and send changes continusly
    listen_and_update_state(client, backend, block_metrics, chain_id)
        .await
        .context("Subscribing to the LogStateUpdate event")?;

    Ok(())
}

#[cfg(test)]
mod eth_client_event_subscription_test {
    use alloy::eips::BlockNumberOrTag;
    use alloy::node_bindings::Anvil;
    use alloy::providers::{Provider, ProviderBuilder};
    use alloy::sol;
    use futures::StreamExt;
    use url::Url;

    sol!(
        #[derive(Debug)]
        #[sol(rpc)]
        SimpleStorage,
        "src/abis/simple_storage.json"
    );

    #[tokio::test]
    async fn test_event_subscription() {
        let anvil = Anvil::new()
            .fork("https://eth.merkle.io")
            .fork_block_number(20395662)
            .try_spawn()
            .expect("issue while forking");
        let rpc_url: Url = anvil.endpoint().parse().expect("issue while parsing");
        let provider = ProviderBuilder::new().on_http(rpc_url.clone());

        let address = anvil.addresses()[0];

        let contract = SimpleStorage::deploy(provider.clone(), "initial value".to_string()).await.unwrap();

        let event = contract.event_filter::<SimpleStorage::ValueChanged>();
        let mut stream = event.watch().await.unwrap().into_stream();

        let num_tx = 3;

        let starting_block_number = provider.get_block_number().await.unwrap();
        for i in 0..num_tx {
            contract.setValue(i.to_string()).from(address).send().await.unwrap().get_receipt().await.unwrap();

            let log = stream.next().await.unwrap().unwrap();

            assert_eq!(log.0.newValue, i.to_string());
            assert_eq!(log.1.block_number.unwrap(), starting_block_number + i + 1);

            let hash = provider
                .get_block_by_number(BlockNumberOrTag::from(starting_block_number + i + 1), false)
                .await
                .unwrap()
                .unwrap()
                .header
                .hash
                .unwrap();
            assert_eq!(log.1.block_hash.unwrap(), hash);
        }
    }
}
