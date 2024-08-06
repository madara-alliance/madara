use crate::client::EthereumClient;
use crate::l1_gas_price::{gas_price_worker, L1GasPrices};
use crate::state_update::state_update_worker;
use dc_db::DeoxysBackend;
use futures::lock::Mutex;
use starknet_types_core::felt::Felt;
use std::sync::Arc;

pub async fn sync(
    backend: &DeoxysBackend,
    eth_client: &EthereumClient,
    chain_id: Felt,
    l1_gas_prices: Arc<Mutex<L1GasPrices>>,
) -> anyhow::Result<()> {
    let state_update_fut = async { state_update_worker(backend, eth_client, chain_id).await };
    let gas_price_fut = async { gas_price_worker(eth_client, l1_gas_prices, true).await };

    tokio::try_join!(state_update_fut, gas_price_fut)?;

    Ok(())
}
