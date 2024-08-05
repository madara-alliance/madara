use crate::client::EthereumClient;
use alloy::eips::BlockNumberOrTag;
use alloy::providers::Provider;
use anyhow::{format_err, Context};
use primitive_types::U256;
use std::num::NonZeroU128;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::sleep;
const DEFAULT_GAS_PRICE_POLL_MS: u64 = 10_000;

#[derive(Clone, PartialEq, Eq)]
pub struct L1GasPrices {
    pub eth_l1_gas_price: NonZeroU128,       // In wei.
    pub strk_l1_gas_price: NonZeroU128,      // In fri.
    pub eth_l1_data_gas_price: NonZeroU128,  // In wei.
    pub strk_l1_data_gas_price: NonZeroU128, // In fri.
    pub last_update_timestamp: u128,
}

impl Default for L1GasPrices {
    fn default() -> Self {
        L1GasPrices {
            eth_l1_gas_price: NonZeroU128::new(10).unwrap(),
            strk_l1_gas_price: NonZeroU128::new(10).unwrap(),
            eth_l1_data_gas_price: NonZeroU128::new(10).unwrap(),
            strk_l1_data_gas_price: NonZeroU128::new(10).unwrap(),
            last_update_timestamp: Default::default(),
        }
    }
}

pub async fn run_worker(
    eth_client: &EthereumClient,
    gas_price: Arc<Mutex<L1GasPrices>>,
    infinite_loop: bool,
) -> anyhow::Result<()> {
    let poll_time = eth_client.gas_price_poll_ms.unwrap_or(DEFAULT_GAS_PRICE_POLL_MS);

    loop {
        match update_gas_price(&eth_client, gas_price.clone()).await {
            Ok(_) => log::trace!("Updated gas prices"),
            Err(e) => log::error!("Failed to update gas prices: {:?}", e),
        }

        let gas_price = gas_price
            .lock()
            .map_err(|e| anyhow::anyhow!("Failed to lock gas price: {:?}", e))
            .context("Setting l1 last confirmed block number")?;
        let last_update_timestamp = gas_price.last_update_timestamp;
        drop(gas_price);
        let current_timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("Failed to get current timestamp")
            .as_millis();

        if current_timestamp - last_update_timestamp > 10 * poll_time as u128 {
            panic!(
                "Gas prices have not been updated for {} ms. Last update was at {}",
                current_timestamp - last_update_timestamp,
                last_update_timestamp
            );
        }

        if !infinite_loop {
            return Ok(());
        }

        sleep(Duration::from_millis(poll_time)).await;
    }
}

async fn update_gas_price(eth_client: &EthereumClient, gas_price: Arc<Mutex<L1GasPrices>>) -> anyhow::Result<()> {
    let fee_history = eth_client.provider.get_fee_history(300, BlockNumberOrTag::Latest, &[]).await?;

    // The RPC responds with 301 elements for some reason. It's also just safer to manually
    // take the last 300. We choose 300 to get average gas caprice for last one hour (300 * 12 sec block
    // time).
    let (_, blob_fee_history_one_hour) =
        fee_history.base_fee_per_blob_gas.split_at(fee_history.base_fee_per_blob_gas.len().max(300) - 300);

    let avg_blob_base_fee =
        blob_fee_history_one_hour.into_iter().sum::<u128>() / blob_fee_history_one_hour.len() as u128;

    let eth_gas_price = fee_history.base_fee_per_blob_gas.last().context("Setting l1 last confirmed block number")?;

    let mut gas_price = gas_price
        .lock()
        .map_err(|e| anyhow::anyhow!("Failed to lock gas price: {:?}", e))
        .context("Setting l1 last confirmed block number")?;

    gas_price.eth_l1_gas_price = NonZeroU128::new(*eth_gas_price).context("Setting l1 last confirmed block number")?;
    gas_price.eth_l1_data_gas_price =
        NonZeroU128::new(avg_blob_base_fee).context("Setting l1 last confirmed block number")?;

    gas_price.last_update_timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis();
    // explicitly dropping gas price here to avoid long waits when fetching the value
    // on the inherent side which would increase block time
    drop(gas_price);

    Ok(())
}
