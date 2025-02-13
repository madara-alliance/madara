use crate::client::SettlementClientTrait;
use anyhow::Context;
use bigdecimal::BigDecimal;
use mc_mempool::{GasPriceProvider, L1DataProvider};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use crate::error::SettlementClientError;
use crate::messaging::L1toL2MessagingEventData;
use futures::Stream;
use mc_analytics::register_gauge_metric_instrument;
use mp_utils::service::ServiceContext;
use opentelemetry::global::Error as OtelError;
use opentelemetry::metrics::Gauge;
use opentelemetry::{global, KeyValue};
use std::time::SystemTime;

#[derive(Clone, Debug)]
pub struct L1BlockMetrics {
    // L1 network metrics
    pub l1_block_number: Gauge<u64>,
    // gas price is also define in sync/metrics/block_metrics.rs but this would be the price from l1
    pub l1_gas_price_wei: Gauge<u64>,
    pub l1_gas_price_strk: Gauge<f64>,
}

impl L1BlockMetrics {
    pub fn register() -> Result<Self, OtelError> {
        let common_scope_attributes = vec![KeyValue::new("crate", "L1 Block")];
        let eth_meter = global::meter_with_version(
            "crates.l1block.opentelemetry",
            Some("0.17"),
            Some("https://opentelemetry.io/schemas/1.2.0"),
            Some(common_scope_attributes.clone()),
        );

        let l1_block_number = register_gauge_metric_instrument(
            &eth_meter,
            "l1_block_number".to_string(),
            "Gauge for madara L1 block number".to_string(),
            "".to_string(),
        );

        let l1_gas_price_wei = register_gauge_metric_instrument(
            &eth_meter,
            "l1_gas_price_wei".to_string(),
            "Gauge for madara L1 gas price in wei".to_string(),
            "".to_string(),
        );

        let l1_gas_price_strk = register_gauge_metric_instrument(
            &eth_meter,
            "l1_gas_price_strk".to_string(),
            "Gauge for madara L1 gas price in strk".to_string(),
            "".to_string(),
        );

        Ok(Self { l1_block_number, l1_gas_price_wei, l1_gas_price_strk })
    }
}

pub async fn gas_price_worker_once<C, S>(
    settlement_client: Arc<Box<dyn SettlementClientTrait<Config = C, StreamType = S>>>,
    l1_gas_provider: &GasPriceProvider,
    gas_price_poll_ms: Duration,
    l1_block_metrics: Arc<L1BlockMetrics>,
) -> Result<(), SettlementClientError>
where
    S: Stream<Item = Result<L1toL2MessagingEventData, SettlementClientError>> + Send + 'static,
{
    match update_gas_price(settlement_client, l1_gas_provider, l1_block_metrics).await {
        Ok(_) => tracing::trace!("Updated gas prices"),
        Err(e) => tracing::error!("Failed to update gas prices: {:?}", e),
    }

    let last_update_timestamp = l1_gas_provider.get_gas_prices_last_update();
    let duration_since_last_update = SystemTime::now()
        .duration_since(last_update_timestamp)
        .context("Failed to calculate time since last update")
        .map_err(SettlementClientError::Other)?;

    let last_update_timestamp = last_update_timestamp
        .duration_since(UNIX_EPOCH)
        .context("SystemTime before UNIX EPOCH!")
        .map_err(SettlementClientError::Other)?
        .as_micros();

    if duration_since_last_update > 10 * gas_price_poll_ms {
        return Err(SettlementClientError::Other(anyhow::anyhow!(
            "Gas prices have not been updated for {} ms. Last update was at {}",
            duration_since_last_update.as_micros(),
            last_update_timestamp
        )));
    }

    Ok(())
}

pub async fn gas_price_worker<C, S>(
    settlement_client: Arc<Box<dyn SettlementClientTrait<Config = C, StreamType = S>>>,
    l1_gas_provider: GasPriceProvider,
    gas_price_poll_ms: Duration,
    mut ctx: ServiceContext,
    l1_block_metrics: Arc<L1BlockMetrics>,
) -> Result<(), SettlementClientError>
where
    S: Stream<Item = Result<L1toL2MessagingEventData, SettlementClientError>> + Send + 'static,
{
    l1_gas_provider.update_last_update_timestamp();
    let mut interval = tokio::time::interval(gas_price_poll_ms);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    while ctx.run_until_cancelled(interval.tick()).await.is_some() {
        gas_price_worker_once(
            Arc::clone(&settlement_client),
            &l1_gas_provider,
            gas_price_poll_ms,
            l1_block_metrics.clone(),
        )
        .await?;
    }

    Ok(())
}

async fn update_gas_price<C, S>(
    settlement_client: Arc<Box<dyn SettlementClientTrait<Config = C, StreamType = S>>>,
    l1_gas_provider: &GasPriceProvider,
    l1_block_metrics: Arc<L1BlockMetrics>,
) -> Result<(), SettlementClientError>
where
    S: Stream<Item = Result<L1toL2MessagingEventData, SettlementClientError>> + Send + 'static,
{
    let (eth_gas_price, avg_blob_base_fee) = settlement_client
        .get_gas_prices()
        .await
        .context("Failed to get gas prices")
        .map_err(SettlementClientError::Other)?;

    l1_gas_provider.update_eth_l1_gas_price(eth_gas_price);
    l1_gas_provider.update_eth_l1_data_gas_price(avg_blob_base_fee);

    // fetch eth/strk price and update
    if let Some(oracle_provider) = &l1_gas_provider.oracle_provider {
        let (eth_strk_price, decimals) = oracle_provider
            .fetch_eth_strk_price()
            .await
            .context("Failed to retrieve ETH/STRK price")
            .map_err(SettlementClientError::Other)?;

        let strk_gas_price = (BigDecimal::new(eth_gas_price.into(), decimals.into())
            / BigDecimal::new(eth_strk_price.into(), decimals.into()))
        .as_bigint_and_exponent();
        let strk_data_gas_price = (BigDecimal::new(avg_blob_base_fee.into(), decimals.into())
            / BigDecimal::new(eth_strk_price.into(), decimals.into()))
        .as_bigint_and_exponent();

        l1_gas_provider.update_strk_l1_gas_price(
            strk_gas_price
                .0
                .to_str_radix(10)
                .parse::<u128>()
                .context("Failed to update STRK L1 gas price")
                .map_err(SettlementClientError::Other)?,
        );
        l1_gas_provider.update_strk_l1_data_gas_price(
            strk_data_gas_price
                .0
                .to_str_radix(10)
                .parse::<u128>()
                .context("Failed to update STRK L1 data gas price")
                .map_err(SettlementClientError::Other)?,
        );
    }

    l1_gas_provider.update_last_update_timestamp();

    // Update block number separately to avoid holding the lock for too long
    update_l1_block_metrics(
        settlement_client
            .get_latest_block_number()
            .await
            .context("Failed to get latest block number")
            .map_err(SettlementClientError::Other)?,
        l1_block_metrics,
        l1_gas_provider,
    )
    .await
    .context("Failed to update L1 block metrics")
    .map_err(SettlementClientError::Other)?;

    Ok(())
}

async fn update_l1_block_metrics(
    block_number: u64,
    l1_block_metrics: Arc<L1BlockMetrics>,
    l1_gas_provider: &GasPriceProvider,
) -> Result<(), SettlementClientError> {
    // Get the current gas price
    let current_gas_price = l1_gas_provider.get_gas_prices();
    let eth_gas_price = current_gas_price.eth_l1_gas_price;

    tracing::debug!("Gas price fetched is: {:?}", eth_gas_price);

    // Update the metrics
    l1_block_metrics.l1_block_number.record(block_number, &[]);
    l1_block_metrics.l1_gas_price_wei.record(eth_gas_price as u64, &[]);

    // We're ignoring l1_gas_price_strk
    Ok(())
}

#[cfg(test)]
mod eth_client_gas_price_worker_test {
    use super::*;
    use crate::eth::eth_client_getter_test::{create_ethereum_client, eth_client};
    use crate::eth::event::EthereumEventStream;
    use crate::eth::{EthereumClient, EthereumClientConfig};
    use httpmock::{MockServer, Regex};
    use mc_mempool::GasPriceProvider;
    use rstest::*;
    use std::time::SystemTime;
    use tokio::task::JoinHandle;
    use tokio::time::{timeout, Duration};

    #[rstest]
    #[tokio::test]
    async fn gas_price_worker_when_infinite_loop_true_works(eth_client: EthereumClient) {
        let l1_gas_provider = GasPriceProvider::new();

        let l1_block_metrics = L1BlockMetrics::register().expect("Failed to register L1 block metrics");

        // Spawn the gas_price_worker in a separate task
        let worker_handle: JoinHandle<Result<(), SettlementClientError>> = tokio::spawn({
            let eth_client = eth_client.clone();
            let l1_gas_provider = l1_gas_provider.clone();
            async move {
                gas_price_worker::<EthereumClientConfig, EthereumEventStream>(
                    Arc::new(Box::new(eth_client)),
                    l1_gas_provider,
                    Duration::from_millis(200),
                    ServiceContext::new_for_testing(),
                    Arc::new(l1_block_metrics),
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

    #[rstest]
    #[tokio::test]
    async fn gas_price_worker_when_infinite_loop_false_works(eth_client: EthereumClient) {
        let l1_gas_provider = GasPriceProvider::new();

        let l1_block_metrics = L1BlockMetrics::register().expect("Failed to register L1 block metrics");

        // Run the worker for a short time
        let worker_handle = gas_price_worker_once::<EthereumClientConfig, EthereumEventStream>(
            Arc::new(Box::new(eth_client)),
            &l1_gas_provider,
            Duration::from_millis(200),
            Arc::new(l1_block_metrics),
        );

        // Wait for the worker to complete
        worker_handle.await.expect("issue with the gas worker");

        // Check if the gas price was updated
        let updated_price = l1_gas_provider.get_gas_prices();
        assert_eq!(updated_price.eth_l1_gas_price, 948082986);
        assert_eq!(updated_price.eth_l1_data_gas_price, 1);
    }

    #[rstest]
    #[tokio::test]
    async fn gas_price_worker_when_gas_price_fix_works(eth_client: EthereumClient) {
        let l1_gas_provider = GasPriceProvider::new();
        l1_gas_provider.update_eth_l1_gas_price(20);
        l1_gas_provider.set_gas_price_sync_enabled(false);

        let l1_block_metrics = L1BlockMetrics::register().expect("Failed to register L1 block metrics");

        // Run the worker for a short time
        let worker_handle = gas_price_worker_once::<EthereumClientConfig, EthereumEventStream>(
            Arc::new(Box::new(eth_client)),
            &l1_gas_provider,
            Duration::from_millis(200),
            Arc::new(l1_block_metrics),
        );

        // Wait for the worker to complete
        worker_handle.await.expect("issue with the gas worker");

        // Check if the gas price was updated
        let updated_price = l1_gas_provider.get_gas_prices();
        assert_eq!(updated_price.eth_l1_gas_price, 20);
        assert_eq!(updated_price.eth_l1_data_gas_price, 1);
    }

    #[rstest]
    #[tokio::test]
    async fn gas_price_worker_when_data_gas_price_fix_works(eth_client: EthereumClient) {
        let l1_gas_provider = GasPriceProvider::new();
        l1_gas_provider.update_eth_l1_data_gas_price(20);
        l1_gas_provider.set_data_gas_price_sync_enabled(false);

        let l1_block_metrics = L1BlockMetrics::register().expect("Failed to register L1 block metrics");

        // Run the worker for a short time
        let worker_handle = gas_price_worker_once::<EthereumClientConfig, EthereumEventStream>(
            Arc::new(Box::new(eth_client)),
            &l1_gas_provider,
            Duration::from_millis(200),
            Arc::new(l1_block_metrics),
        );

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
        let eth_client = create_ethereum_client(Some(&addr));
        let l1_block_metrics = L1BlockMetrics::register().expect("Failed to register L1 block metrics");

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
            gas_price_worker::<EthereumClientConfig, EthereumEventStream>(
                Arc::new(Box::new(eth_client)),
                l1_gas_provider.clone(),
                Duration::from_millis(200),
                ServiceContext::new_for_testing(),
                Arc::new(l1_block_metrics),
            ),
        )
        .await;

        match result {
            Ok(Ok(_)) => panic!("Expected gas_price_worker to fail, but it succeeded"),
            Ok(Err(err)) => match err {
                SettlementClientError::Other(e) => {
                    let error_msg = e.to_string();
                    let re =
                        Regex::new(r"Gas prices have not been updated for \d+ ms\. Last update was at \d+").unwrap();
                    assert!(re.is_match(&error_msg), "Error message did not match expected format. Got: {}", error_msg);
                }
                other => panic!("Expected Other error variant, got: {:?}", other),
            },
            Err(err) => panic!("gas_price_worker timed out: {err}"),
        }

        // Verify that the mock was called
        mock.assert();
    }

    #[rstest]
    #[tokio::test]
    async fn update_gas_price_works(eth_client: EthereumClient) {
        let l1_gas_provider = GasPriceProvider::new();

        l1_gas_provider.update_last_update_timestamp();
        let l1_block_metrics = L1BlockMetrics::register().expect("Failed to register L1 block metrics");

        // Update gas prices
        update_gas_price::<EthereumClientConfig, EthereumEventStream>(
            Arc::new(Box::new(eth_client)),
            &l1_gas_provider,
            Arc::new(l1_block_metrics),
        )
        .await
        .expect("Failed to update gas prices");

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
