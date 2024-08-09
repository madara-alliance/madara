use crate::client::EthereumClient;
use crate::l1_gas_price::gas_price_worker;
use crate::state_update::state_update_worker;
use dc_mempool::L1DataProvider;
use starknet_types_core::felt::Felt;

use dc_db::DeoxysBackend;

use std::sync::Arc;

pub async fn l1_sync_worker(
    backend: &DeoxysBackend,
    eth_client: &EthereumClient,
    chain_id: Felt,
    l1_data_provider: Arc<dyn L1DataProvider>,
    gas_price_sync_disabled: bool,
    gas_price_poll_ms: u64,
) -> anyhow::Result<()> {
    let state_update_fut = async { state_update_worker(backend, eth_client, chain_id).await };

    if gas_price_sync_disabled {
        // Only run the state update worker if gas price sync is disabled
        state_update_fut.await?;
    } else {
        // Run both workers if gas price sync is enabled
        let gas_price_fut = async { gas_price_worker(eth_client, l1_data_provider, true, gas_price_poll_ms).await };
        tokio::try_join!(state_update_fut, gas_price_fut)?;
    }

    Ok(())
}
