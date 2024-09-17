use crate::client::EthereumClient;
use alloy::eips::BlockNumberOrTag;
use alloy::providers::Provider;
use anyhow::Context;
use mc_mempool::{GasPriceProvider, L1DataProvider};
use std::time::{Duration, UNIX_EPOCH};

use mp_utils::wait_or_graceful_shutdown;
use std::time::SystemTime;

pub async fn gas_price_worker_once(
    eth_client: &EthereumClient,
    l1_gas_provider: GasPriceProvider,
    gas_price_poll_ms: Duration,
) -> anyhow::Result<()> {
    match update_gas_price(eth_client, l1_gas_provider.clone()).await {
        Ok(_) => log::trace!("Updated gas prices"),
        Err(e) => log::error!("Failed to update gas prices: {:?}", e),
    }

    let last_update_timestamp = l1_gas_provider.get_gas_prices_last_update();
    let duration_since_last_update = SystemTime::now().duration_since(last_update_timestamp)?;
    let last_update_timestemp =
        last_update_timestamp.duration_since(UNIX_EPOCH).expect("SystemTime before UNIX EPOCH!").as_micros();
    if duration_since_last_update > 10 * gas_price_poll_ms {
        anyhow::bail!(
            "Gas prices have not been updated for {} ms. Last update was at {}",
            duration_since_last_update.as_micros(),
            last_update_timestemp
        );
    }

    Ok(())
}
pub async fn gas_price_worker(
    eth_client: &EthereumClient,
    l1_gas_provider: GasPriceProvider,
    gas_price_poll_ms: Duration,
) -> anyhow::Result<()> {
    l1_gas_provider.update_last_update_timestamp();
    let mut interval = tokio::time::interval(gas_price_poll_ms);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    while wait_or_graceful_shutdown(interval.tick()).await.is_some() {
        gas_price_worker_once(eth_client, l1_gas_provider.clone(), gas_price_poll_ms).await?;
    }
    Ok(())
}

async fn update_gas_price(eth_client: &EthereumClient, l1_gas_provider: GasPriceProvider) -> anyhow::Result<()> {
    let block_number = eth_client.get_latest_block_number().await?;
    let fee_history = eth_client.provider.get_fee_history(300, BlockNumberOrTag::Number(block_number), &[]).await?;

    // The RPC responds with 301 elements for some reason. It's also just safer to manually
    // take the last 300. We choose 300 to get average gas caprice for last one hour (300 * 12 sec block
    // time).
    let (_, blob_fee_history_one_hour) =
        fee_history.base_fee_per_blob_gas.split_at(fee_history.base_fee_per_blob_gas.len().max(300) - 300);

    let avg_blob_base_fee = blob_fee_history_one_hour.iter().sum::<u128>() / blob_fee_history_one_hour.len() as u128;

    let eth_gas_price = fee_history.base_fee_per_gas.last().context("Getting eth gas price")?;

    l1_gas_provider.update_eth_l1_gas_price(*eth_gas_price);
    l1_gas_provider.update_eth_l1_data_gas_price(avg_blob_base_fee);

    l1_gas_provider.update_last_update_timestamp();

    // Update block number separately to avoid holding the lock for too long
    update_l1_block_metrics(eth_client, l1_gas_provider).await?;

    Ok(())
}

async fn update_l1_block_metrics(eth_client: &EthereumClient, l1_gas_provider: GasPriceProvider) -> anyhow::Result<()> {
    // Get the latest block number
    let latest_block_number = eth_client.get_latest_block_number().await?;

    // Get the current gas price
    let current_gas_price = l1_gas_provider.get_gas_prices();
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
    use httpmock::{MockServer, Regex};
    use mc_mempool::GasPriceProvider;
    use serial_test::serial;
    use std::time::SystemTime;
    use tokio::task::JoinHandle;
    use tokio::time::{timeout, Duration};
    const ANOTHER_ANVIL_PORT: u16 = 8546;
    const L1_BLOCK_NUMBER: u64 = 20395662;
    const FORK_URL: &str = "https://eth.merkle.io";

    #[serial]
    #[tokio::test]
    async fn gas_price_worker_when_infinite_loop_true_works() {
        let anvil = Anvil::new()
            .fork(FORK_URL)
            .fork_block_number(L1_BLOCK_NUMBER)
            .port(ANOTHER_ANVIL_PORT)
            .try_spawn()
            .expect("issue while forking for the anvil");
        let eth_client = create_ethereum_client(Some(anvil.endpoint().as_str()));
        let l1_gas_provider = GasPriceProvider::new();

        // Spawn the gas_price_worker in a separate task
        let worker_handle: JoinHandle<anyhow::Result<()>> = tokio::spawn({
            let eth_client = eth_client.clone();
            let l1_gas_provider = l1_gas_provider.clone();
            async move { gas_price_worker(&eth_client, l1_gas_provider, Duration::from_millis(200)).await }
        });

        // Wait for a short duration to allow the worker to run
        tokio::time::sleep(Duration::from_secs(10)).await;

        // Abort the worker task
        worker_handle.abort();

        // Wait for the worker to finish (it should be aborted quickly)
        let timeout_duration = Duration::from_secs(2);
        let result = timeout(timeout_duration, worker_handle).await;

        match result {
            Ok(Ok(_)) => println!("Gas price worker completed successfully"),
            Ok(Err(e)) => println!("Gas price worker encountered an error: {:?}", e),
            Err(_) => println!("Gas price worker timed out"),
        }

        // Check if the gas price was updated
        let updated_price = l1_gas_provider.get_gas_prices();
        assert_eq!(updated_price.eth_l1_gas_price, 948082986);
        assert_eq!(updated_price.eth_l1_data_gas_price, 1);
    }

    #[serial]
    #[tokio::test]
    async fn gas_price_worker_when_infinite_loop_false_works() {
        let anvil = Anvil::new()
            .fork(FORK_URL)
            .fork_block_number(L1_BLOCK_NUMBER)
            .port(ANOTHER_ANVIL_PORT)
            .try_spawn()
            .expect("issue while forking for the anvil");
        let eth_client = create_ethereum_client(Some(anvil.endpoint().as_str()));
        let l1_gas_provider = GasPriceProvider::new();

        // Run the worker for a short time
        let worker_handle = gas_price_worker_once(&eth_client, l1_gas_provider.clone(), Duration::from_millis(200));

        // Wait for the worker to complete
        worker_handle.await.expect("issue with the gas worker");

        // Check if the gas price was updated
        let updated_price = l1_gas_provider.get_gas_prices();
        assert_eq!(updated_price.eth_l1_gas_price, 948082986);
        assert_eq!(updated_price.eth_l1_data_gas_price, 1);
    }

    #[serial]
    #[tokio::test]
    async fn gas_price_worker_when_eth_fee_history_fails_should_fails() {
        let mock_server = MockServer::start();
        let addr = format!("http://{}", mock_server.address());
        let eth_client = create_ethereum_client(Some(&addr));

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

        let l1_gas_provider = GasPriceProvider::new();

        l1_gas_provider.update_last_update_timestamp();

        let timeout_duration = Duration::from_secs(10);

        let result = timeout(
            timeout_duration,
            gas_price_worker(&eth_client, l1_gas_provider.clone(), Duration::from_millis(200)),
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
            Err(err) => panic!("gas_price_worker timed out: {err}"),
        }

        // Verify that the mock was called
        mock.assert();
    }

    #[serial]
    #[tokio::test]
    async fn update_gas_price_works() {
        let anvil = Anvil::new()
            .fork(FORK_URL)
            .fork_block_number(L1_BLOCK_NUMBER)
            .port(ANOTHER_ANVIL_PORT)
            .try_spawn()
            .expect("issue while forking for the anvil");
        let eth_client = create_ethereum_client(Some(anvil.endpoint().as_str()));
        let l1_gas_provider = GasPriceProvider::new();

        l1_gas_provider.update_last_update_timestamp();

        // Update gas prices
        update_gas_price(&eth_client, l1_gas_provider.clone()).await.expect("Failed to update gas prices");

        // Access the updated gas prices
        let updated_prices = l1_gas_provider.get_gas_prices();

        assert_eq!(
            updated_prices.eth_l1_gas_price, 948082986,
            "ETH L1 gas price should be 948082986 in test environment"
        );

        assert_eq!(updated_prices.eth_l1_data_gas_price, 1, "ETH L1 data gas price should be 1 in test environment");

        // Verify that the last update timestamp is recent

        let last_update_timestamp = l1_gas_provider.get_gas_prices_last_update();

        let time_since_last_update =
            SystemTime::now().duration_since(last_update_timestamp).expect("issue while getting the time");

        assert!(time_since_last_update.as_secs() < 60, "Last update timestamp should be within the last minute");
    }
}
