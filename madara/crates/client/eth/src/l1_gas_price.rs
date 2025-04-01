use crate::client::EthereumClient;
use alloy::eips::BlockNumberOrTag;
use alloy::providers::Provider;
use anyhow::Context;
use bigdecimal::BigDecimal;
use mc_mempool::{GasPriceProvider, L1DataProvider};
use std::{
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};

use mp_utils::service::ServiceContext;
use std::time::SystemTime;

pub async fn gas_price_worker_once(
    eth_client: &EthereumClient,
    l1_gas_provider: &GasPriceProvider,
    gas_price_poll_ms: Duration,
) -> anyhow::Result<()> {
    match update_gas_price(eth_client, l1_gas_provider).await {
        Ok(_) => tracing::trace!("Updated gas prices"),
        Err(e) => tracing::error!("Failed to update gas prices: {:?}", e),
    }

    let last_update_timestamp = l1_gas_provider.get_gas_prices_last_update();
    let duration_since_last_update = SystemTime::now().duration_since(last_update_timestamp)?;

    let last_update_timestamp =
        last_update_timestamp.duration_since(UNIX_EPOCH).expect("SystemTime before UNIX EPOCH!").as_micros();
    if duration_since_last_update > 10 * gas_price_poll_ms {
        anyhow::bail!(
            "Gas prices have not been updated for {} ms. Last update was at {}",
            duration_since_last_update.as_micros(),
            last_update_timestamp
        );
    }

    anyhow::Ok(())
}
pub async fn gas_price_worker(
    eth_client: Arc<EthereumClient>,
    l1_gas_provider: GasPriceProvider,
    gas_price_poll_ms: Duration,
    mut ctx: ServiceContext,
) -> anyhow::Result<()> {
    l1_gas_provider.update_last_update_timestamp();
    let mut interval = tokio::time::interval(gas_price_poll_ms);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    while ctx.run_until_cancelled(interval.tick()).await.is_some() {
        gas_price_worker_once(&eth_client, &l1_gas_provider, gas_price_poll_ms).await?;
    }

    anyhow::Ok(())
}

async fn update_gas_price(eth_client: &EthereumClient, l1_gas_provider: &GasPriceProvider) -> anyhow::Result<()> {
    let block_number = eth_client.get_latest_block_number().await?;
    let fee_history = eth_client.provider.get_fee_history(300, BlockNumberOrTag::Number(block_number), &[]).await?;

    // The RPC responds with 301 elements for some reason. It's also just safer to manually
    // take the last 300. We choose 300 to get average gas caprice for last one hour (300 * 12 sec block
    // time).
    let (_, blob_fee_history_one_hour) =
        fee_history.base_fee_per_blob_gas.split_at(fee_history.base_fee_per_blob_gas.len().max(300) - 300);

    let avg_blob_base_fee = if !blob_fee_history_one_hour.is_empty() {
        blob_fee_history_one_hour.iter().sum::<u128>() / blob_fee_history_one_hour.len() as u128
    } else {
        0 // in case blob_fee_history_one_hour has 0 length
    };

    let eth_gas_price = fee_history.base_fee_per_gas.last().context("Getting eth gas price")?;

    l1_gas_provider.update_eth_l1_gas_price(*eth_gas_price);
    l1_gas_provider.update_eth_l1_data_gas_price(avg_blob_base_fee);

    // fetch eth/strk price and update
    if let Some(oracle_provider) = &l1_gas_provider.oracle_provider {
        let (eth_strk_price, decimals) =
            oracle_provider.fetch_eth_strk_price().await.context("failed to retrieve ETH/STRK price")?;
        let strk_gas_price = (BigDecimal::new((*eth_gas_price).into(), decimals.into())
            / BigDecimal::new(eth_strk_price.into(), decimals.into()))
        .as_bigint_and_exponent();
        let strk_data_gas_price = (BigDecimal::new(avg_blob_base_fee.into(), decimals.into())
            / BigDecimal::new(eth_strk_price.into(), decimals.into()))
        .as_bigint_and_exponent();

        l1_gas_provider.update_strk_l1_gas_price(
            strk_gas_price.0.to_str_radix(10).parse::<u128>().context("failed to update strk l1 gas price")?,
        );
        l1_gas_provider.update_strk_l1_data_gas_price(
            strk_data_gas_price
                .0
                .to_str_radix(10)
                .parse::<u128>()
                .context("failed to update strk l1 data gas price")?,
        );
    }

    l1_gas_provider.update_last_update_timestamp();

    // Update block number separately to avoid holding the lock for too long
    update_l1_block_metrics(eth_client, l1_gas_provider).await?;

    Ok(())
}

async fn update_l1_block_metrics(
    eth_client: &EthereumClient,
    l1_gas_provider: &GasPriceProvider,
) -> anyhow::Result<()> {
    // Get the latest block number
    let latest_block_number = eth_client.get_latest_block_number().await?;

    // Get the current gas price
    let current_gas_price = l1_gas_provider.get_gas_prices();
    let eth_gas_price = current_gas_price.eth_l1_gas_price;

    tracing::debug!("Gas price fetched is: {:?}", eth_gas_price);

    // Update the metrics

    eth_client.l1_block_metrics.l1_block_number.record(latest_block_number, &[]);
    eth_client.l1_block_metrics.l1_gas_price_wei.record(eth_gas_price as u64, &[]);

    // We're ignoring l1_gas_price_strk

    Ok(())
}

#[cfg(test)]
mod eth_client_gas_price_worker_test {
    use super::*;
    use crate::client::eth_client_getter_test::{create_ethereum_client, get_anvil_url};
    use httpmock::{MockServer, Regex};
    use mc_mempool::GasPriceProvider;
    use std::time::SystemTime;
    use tokio::task::JoinHandle;
    use tokio::time::{timeout, Duration};

    #[tokio::test]
    async fn gas_price_worker_when_infinite_loop_true_works() {
        let eth_client = create_ethereum_client(get_anvil_url());
        let l1_gas_provider = GasPriceProvider::new();

        // Spawn the gas_price_worker in a separate task
        let worker_handle: JoinHandle<anyhow::Result<()>> = tokio::spawn({
            let eth_client = eth_client.clone();
            let l1_gas_provider = l1_gas_provider.clone();
            async move {
                gas_price_worker(
                    Arc::new(eth_client),
                    l1_gas_provider,
                    Duration::from_millis(200),
                    ServiceContext::new_for_testing(),
                )
                .await
            }
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

    #[tokio::test]
    async fn gas_price_worker_when_infinite_loop_false_works() {
        let eth_client = create_ethereum_client(get_anvil_url());
        let l1_gas_provider = GasPriceProvider::new();

        // Run the worker for a short time
        let worker_handle = gas_price_worker_once(&eth_client, &l1_gas_provider, Duration::from_millis(200));

        // Wait for the worker to complete
        worker_handle.await.expect("issue with the gas worker");

        // Check if the gas price was updated
        let updated_price = l1_gas_provider.get_gas_prices();
        assert_eq!(updated_price.eth_l1_gas_price, 948082986);
        assert_eq!(updated_price.eth_l1_data_gas_price, 1);
    }

    #[tokio::test]
    async fn gas_price_worker_when_gas_price_fix_works() {
        let eth_client = create_ethereum_client(get_anvil_url());
        let l1_gas_provider = GasPriceProvider::new();
        l1_gas_provider.update_eth_l1_gas_price(20);
        l1_gas_provider.set_gas_price_sync_enabled(false);

        // Run the worker for a short time
        let worker_handle = gas_price_worker_once(&eth_client, &l1_gas_provider, Duration::from_millis(200));

        // Wait for the worker to complete
        worker_handle.await.expect("issue with the gas worker");

        // Check if the gas price was updated
        let updated_price = l1_gas_provider.get_gas_prices();
        assert_eq!(updated_price.eth_l1_gas_price, 20);
        assert_eq!(updated_price.eth_l1_data_gas_price, 1);
    }

    #[tokio::test]
    async fn gas_price_worker_when_data_gas_price_fix_works() {
        let eth_client = create_ethereum_client(get_anvil_url());
        let l1_gas_provider = GasPriceProvider::new();
        l1_gas_provider.update_eth_l1_data_gas_price(20);
        l1_gas_provider.set_data_gas_price_sync_enabled(false);

        // Run the worker for a short time
        let worker_handle = gas_price_worker_once(&eth_client, &l1_gas_provider, Duration::from_millis(200));

        // Wait for the worker to complete
        worker_handle.await.expect("issue with the gas worker");

        // Check if the gas price was updated
        let updated_price = l1_gas_provider.get_gas_prices();
        assert_eq!(updated_price.eth_l1_gas_price, 948082986);
        assert_eq!(updated_price.eth_l1_data_gas_price, 20);
    }

    #[tokio::test]
    async fn gas_price_worker_when_eth_fee_history_fails_should_fails() {
        let mock_server = MockServer::start();
        let addr = format!("http://{}", mock_server.address());
        let eth_client = create_ethereum_client(addr);

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
            when.method("POST")
                .path("/")
                .json_body_obj(&serde_json::json!({"id":0,"jsonrpc":"2.0","method":"eth_blockNumber"}));
            then.status(200).json_body_obj(&serde_json::json!({"jsonrpc":"2.0","id":1,"result":"0x0137368e"}));
        });

        let l1_gas_provider = GasPriceProvider::new();

        l1_gas_provider.update_last_update_timestamp();

        let timeout_duration = Duration::from_secs(10);

        let result = timeout(
            timeout_duration,
            gas_price_worker(
                Arc::new(eth_client),
                l1_gas_provider.clone(),
                Duration::from_millis(200),
                ServiceContext::new_for_testing(),
            ),
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

    #[tokio::test]
    async fn update_gas_price_works() {
        let eth_client = create_ethereum_client(get_anvil_url());
        let l1_gas_provider = GasPriceProvider::new();

        l1_gas_provider.update_last_update_timestamp();

        // Update gas prices
        update_gas_price(&eth_client, &l1_gas_provider).await.expect("Failed to update gas prices");

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
