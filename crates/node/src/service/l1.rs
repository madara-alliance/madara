use alloy::primitives::Address;
use anyhow::Context;
use starknet_types_core::felt::Felt;
use url::Url;
use dc_db::DeoxysBackend;
use dc_eth::config::{EthereumClient};
use dc_eth::client::update_l1;
use dc_sync::metrics::block_metrics::BlockMetrics;

pub struct L1Service();

impl L1Service {
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
        let start_block = client.get_last_event_block_number().await.context("Retrieving the last event block number")?;
        client
            .listen_and_update_state(backend, start_block, block_metrics, chain_id)
            .await
            .context("Subscribing to the LogStateUpdate event")?;

        Ok(())
    }
}