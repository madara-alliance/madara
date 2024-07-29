use alloy::primitives::Address;
use anyhow::Context;
use dc_db::DeoxysBackend;
use dc_metrics::block_metrics::block_metrics::BlockMetrics;
use dp_convert::ToFelt;
use dp_transactions::TEST_CHAIN_ID;
use dp_utils::channel_wait_or_graceful_shutdown;
use futures::StreamExt;
use starknet_types_core::felt::Felt;
use url::Url;

use crate::{
    client::{EthereumClient, StarknetCoreContract},
    config::L1StateUpdate,
    utils::{convert_log_state_update, trim_hash},
};

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
    let initial_state = EthereumClient::get_initial_state(&client).await.context("Getting initial ethereum state")?;
    update_l1(backend, initial_state, block_metrics.clone(), chain_id)?;

    // Listen to LogStateUpdate (0x77552641) update and send changes continusly
    listen_and_update_state(client, backend, block_metrics, chain_id)
        .await
        .context("Subscribing to the LogStateUpdate event")?;

    Ok(())
}
