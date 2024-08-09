use crate::client::EthereumClient;
use alloy::eips::BlockNumberOrTag;
use alloy::providers::Provider;
use anyhow::Context;
use dc_mempool::L1DataProvider;
use futures::lock::Mutex;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

use lazy_static;
use std::time::{SystemTime, UNIX_EPOCH};

lazy_static::lazy_static! {
    static ref LAST_UPDATE_TIMESTAMP: Arc<Mutex<u128>> = Arc::new(Mutex::new(0));
}

// Function to update the last update timestamp
pub async fn update_last_update_timestamp() {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).expect("Time went backwards").as_millis();
    let mut timestamp = LAST_UPDATE_TIMESTAMP.lock().await;
    *timestamp = now;
}

// Function to get the last update timestamp
pub async fn get_last_update_timestamp() -> u128 {
    *LAST_UPDATE_TIMESTAMP.lock().await
}

pub async fn gas_price_worker(
    eth_client: &EthereumClient,
    l1_data_provider: Arc<dyn L1DataProvider>,
    infinite_loop: bool,
    gas_price_poll_ms: u64,
) -> anyhow::Result<()> {
    update_last_update_timestamp().await;
    loop {
        match update_gas_price(eth_client, Arc::clone(&l1_data_provider)).await {
            Ok(_) => log::trace!("Updated gas prices"),
            Err(e) => log::error!("Failed to update gas prices: {:?}", e),
        }

        let last_update_timestamp = get_last_update_timestamp().await;
        let current_timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("Failed to get current timestamp")
            .as_millis();

        if current_timestamp - last_update_timestamp > 10 * gas_price_poll_ms as u128 {
            panic!(
                "Gas prices have not been updated for {} ms. Last update was at {}",
                current_timestamp - last_update_timestamp,
                last_update_timestamp
            );
        }

        if !infinite_loop {
            return Ok(());
        }

        sleep(Duration::from_millis(gas_price_poll_ms)).await;
    }
}

async fn update_gas_price(
    eth_client: &EthereumClient,
    l1_data_provider: Arc<dyn L1DataProvider>,
) -> anyhow::Result<()> {
    let block_number = eth_client.get_latest_block_number().await?;
    let fee_history = eth_client.provider.get_fee_history(300, BlockNumberOrTag::Number(block_number), &[]).await?;

    // The RPC responds with 301 elements for some reason. It's also just safer to manually
    // take the last 300. We choose 300 to get average gas caprice for last one hour (300 * 12 sec block
    // time).
    let (_, blob_fee_history_one_hour) =
        fee_history.base_fee_per_blob_gas.split_at(fee_history.base_fee_per_blob_gas.len().max(300) - 300);

    let avg_blob_base_fee = blob_fee_history_one_hour.iter().sum::<u128>() / blob_fee_history_one_hour.len() as u128;

    let eth_gas_price = fee_history.base_fee_per_gas.last().context("Getting eth gas price")?;

    l1_data_provider.update_eth_l1_gas_price(*eth_gas_price).await;
    l1_data_provider.update_eth_l1_data_gas_price(avg_blob_base_fee).await;

    update_last_update_timestamp().await;

    // Update block number separately to avoid holding the lock for too long
    update_l1_block_metrics(eth_client, l1_data_provider).await?;

    Ok(())
}

async fn update_l1_block_metrics(
    eth_client: &EthereumClient,
    l1_data_provider: Arc<dyn L1DataProvider>,
) -> anyhow::Result<()> {
    // Get the latest block number
    let latest_block_number = eth_client.get_latest_block_number().await?;

    // Get the current gas price
    let current_gas_price = l1_data_provider.get_gas_prices().await;
    let eth_gas_price = current_gas_price.eth_l1_gas_price;

    // Update the metrics
    eth_client.l1_block_metrics.l1_block_number.set(latest_block_number as f64);
    eth_client.l1_block_metrics.l1_gas_price_wei.set(eth_gas_price as f64);

    // We're ignoring l1_gas_price_strk

    Ok(())
}

#[cfg(test)]
mod eth_client_gas_price_worker_test {
    use super::*;
    use crate::client::eth_client_getter_test::create_ethereum_client;
    use alloy::node_bindings::Anvil;
    use dc_mempool::GasPriceProvider;
    use futures::future::FutureExt;
    use httpmock::{MockServer, Regex};
    use rstest::*;
    use std::panic::AssertUnwindSafe;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tokio::time::{timeout, Duration};

    #[fixture]
    #[once]
    pub fn eth_client_with_mock() -> (MockServer, EthereumClient) {
        let server = MockServer::start();
        let addr = format!("http://{}", server.address());
        let eth_client = create_ethereum_client(Some(&addr));
        (server, eth_client)
    }

    #[fixture]
    #[once]
    pub fn eth_client() -> EthereumClient {
        create_ethereum_client(None)
    }

    #[tokio::test]
    async fn gas_price_worker_when_infinite_loop_true_works() {
        let anvil = Anvil::new()
            .fork("https://eth.merkle.io")
            .fork_block_number(20395662)
            .try_spawn()
            .expect("issue while forking for the anvil");
        let eth_client = create_ethereum_client(Some(anvil.endpoint().as_str()));
        let l1_data_provider: Arc<dyn L1DataProvider> = Arc::new(GasPriceProvider::new());

        // Run the worker for a short time
        let worker_handle = gas_price_worker(&eth_client, l1_data_provider.clone(), false, 20_u64);

        // Wait for the worker to complete
        worker_handle.await.expect("issue with the worker");

        let timeout_duration = Duration::from_secs(5);

        let result =
            timeout(timeout_duration, gas_price_worker(&eth_client, Arc::clone(&l1_data_provider), true, 20_u64)).await;

        match result {
            Ok(Ok(_)) => println!("Gas price worker completed successfully"),
            Ok(Err(e)) => println!("Gas price worker encountered an error: {:?}", e),
            Err(_) => println!("Gas price worker timed out"),
        }

        // Check if the gas price was updated
        let updated_price = l1_data_provider.get_gas_prices().await;
        assert_eq!(updated_price.eth_l1_gas_price, 948082986);
        assert_eq!(updated_price.eth_l1_data_gas_price, 1);
    }

    #[rstest]
    #[tokio::test]
    async fn gas_price_worker_when_infinite_loop_false_works(eth_client: &'static EthereumClient) {
        let l1_data_provider: Arc<dyn L1DataProvider> = Arc::new(GasPriceProvider::new());

        // Run the worker for a short time
        let worker_handle = gas_price_worker(eth_client, l1_data_provider.clone(), false, 20_u64);

        // Wait for the worker to complete
        worker_handle.await.expect("issue with the gas worker");

        // Check if the gas price was updated
        let updated_price = l1_data_provider.get_gas_prices().await;
        assert_eq!(updated_price.eth_l1_gas_price, 948082986);
        assert_eq!(updated_price.eth_l1_data_gas_price, 1);
    }

    #[rstest]
    #[tokio::test]
    async fn gas_price_worker_when_eth_fee_history_fails_should_fails(
        eth_client_with_mock: &'static (MockServer, EthereumClient),
    ) {
        let (mock_server, eth_client) = eth_client_with_mock;

        let mock = mock_server.mock(|when, then| {
            when.method("POST").path("/").json_body_obj(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "eth_feeHistory",
                "params": ["0x12c", "0x137368e", []],
                "id": 1
            }));
            then.status(500).json_body_obj(&serde_json::json!({
                "jsonrpc": "2.0",
                "error": {
                    "code": -32000,
                    "message": "Internal Server Error"
                },
                "id": 1
            }));
        });

        mock_server.mock(|when, then| {
            when.method("POST").path("/").json_body_obj(&serde_json::json!({"id":0,"jsonrpc":"2.0","method":"eth_blockNumber"}));
            then.status(200).json_body_obj(&serde_json::json!({"jsonrpc":"2.0","id":1,"result":"0x0137368e"}                                                                                                                                                                         ));
        });

        let l1_data_provider: Arc<dyn L1DataProvider> = Arc::new(GasPriceProvider::new());

        update_last_update_timestamp().await;

        let timeout_duration = Duration::from_secs(5);

        let result = timeout(
            timeout_duration,
            AssertUnwindSafe(gas_price_worker(eth_client, l1_data_provider, true, 20_u64)).catch_unwind(),
        )
        .await;

        match result {
            Ok(Ok(_)) => panic!("Expected gas_price_worker to panic, but it didn't"),
            Ok(Err(panic_msg)) => {
                if let Some(panic_msg) = panic_msg.downcast_ref::<String>() {
                    let re =
                        Regex::new(r"Gas prices have not been updated for \d+ ms\. Last update was at \d+").unwrap();
                    assert!(re.is_match(panic_msg), "Panic message did not match expected format. Got: {}", panic_msg);
                } else {
                    panic!("Panic occurred, but message was not a string");
                }
            }
            Err(_) => panic!("gas_price_worker timed out"),
        }

        // Verify that the mock was called
        mock.assert();
    }

    #[rstest]
    #[tokio::test]
    async fn update_gas_price_works(eth_client: &'static EthereumClient) {
        let l1_data_provider: Arc<dyn L1DataProvider> = Arc::new(GasPriceProvider::new());

        update_last_update_timestamp().await;

        // Update gas prices
        update_gas_price(eth_client, l1_data_provider.clone()).await.expect("Failed to update gas prices");

        // Access the updated gas prices
        let updated_prices = l1_data_provider.get_gas_prices().await;

        assert_eq!(
            updated_prices.eth_l1_gas_price, 948082986,
            "ETH L1 gas price should be 948082986 in test environment"
        );

        assert_eq!(updated_prices.eth_l1_data_gas_price, 1, "ETH L1 data gas price should be 1 in test environment");

        // Verify that the last update timestamp is recent
        let now = SystemTime::now().duration_since(UNIX_EPOCH).expect("Time went backwards").as_secs() as u128;

        let last_update_timestamp = get_last_update_timestamp().await;
        assert!(last_update_timestamp > now - 60, "Last update timestamp should be within the last minute");
    }
}
