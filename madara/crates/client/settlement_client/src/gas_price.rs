use crate::client::SettlementClientTrait;
use mc_db::MadaraBackend;
use mp_block::L1GasQuote;
use mp_convert::f64_to_u128_fixed;
use mp_oracle::Oracle;
use std::sync::Arc;
use std::time::Duration;

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

#[derive(Clone)]
pub struct GasPriceProviderConfig {
    pub fix_gas_price: Option<u128>,
    pub fix_data_gas_price: Option<u128>,
    pub fix_strk_per_eth: Option<(u128, u32)>,
    pub oracle_provider: Option<Arc<dyn Oracle>>,
    pub poll_interval: Duration,
}

impl GasPriceProviderConfig {
    pub fn all_is_fixed(&self) -> bool {
        self.fix_gas_price.is_some() && self.fix_data_gas_price.is_some() && self.fix_strk_per_eth.is_some()
    }
}

impl From<&GasPriceProviderConfig> for L1GasQuote {
    fn from(value: &GasPriceProviderConfig) -> Self {
        Self {
            l1_gas_price: value.fix_gas_price.unwrap_or_default(),
            l1_data_gas_price: value.fix_data_gas_price.unwrap_or_default(),
            strk_per_eth: value.fix_strk_per_eth.unwrap_or((1, 1)),
        }
    }
}

pub struct GasPriceProviderConfigBuilder {
    fix_gas_price: Option<u128>,
    fix_data_gas_price: Option<u128>,
    fix_strk_per_eth: Option<(u128, u32)>,
    oracle_provider: Option<Arc<dyn Oracle>>,
    poll_interval: Duration,
}

impl Default for GasPriceProviderConfigBuilder {
    fn default() -> Self {
        Self {
            fix_gas_price: None,
            fix_data_gas_price: None,
            fix_strk_per_eth: None,
            oracle_provider: None,
            poll_interval: Duration::from_secs(10),
        }
    }
}

impl GasPriceProviderConfigBuilder {
    pub fn with_fix_gas_price(mut self, value: u128) -> Self {
        self.fix_gas_price = Some(value);
        self
    }

    pub fn set_fix_gas_price(&mut self, value: u128) {
        self.fix_gas_price = Some(value);
    }

    pub fn with_fix_data_gas_price(mut self, value: u128) -> Self {
        self.fix_data_gas_price = Some(value);
        self
    }

    pub fn set_fix_data_gas_price(&mut self, value: u128) {
        self.fix_data_gas_price = Some(value);
    }

    pub fn with_fix_strk_per_eth(mut self, value: f64) -> Self {
        self.fix_strk_per_eth = Some(f64_to_u128_fixed(value));
        self
    }

    pub fn set_fix_strk_per_eth(&mut self, value: f64) {
        self.fix_strk_per_eth = Some(f64_to_u128_fixed(value));
    }

    pub fn with_oracle_provider(mut self, provider: Arc<dyn Oracle>) -> Self {
        self.oracle_provider = Some(provider);
        self
    }

    pub fn set_oracle_provider(&mut self, provider: Arc<dyn Oracle>) {
        self.oracle_provider = Some(provider);
    }

    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    pub fn set_poll_interval(&mut self, interval: Duration) {
        self.poll_interval = interval;
    }

    pub fn build(self) -> anyhow::Result<GasPriceProviderConfig> {
        let needs_oracle = self.fix_strk_per_eth.is_none();
        if needs_oracle && self.oracle_provider.is_none() {
            // This check is commented out because with the current design, we need to create an L1SyncService with gas price even if we didn't need.
            // return Err(anyhow::anyhow!("Oracle provider must be set if no fix_strk_per_eth is provided"));
        }

        Ok(GasPriceProviderConfig {
            fix_gas_price: self.fix_gas_price,
            fix_data_gas_price: self.fix_data_gas_price,
            fix_strk_per_eth: self.fix_strk_per_eth,
            oracle_provider: self.oracle_provider,
            poll_interval: self.poll_interval,
        })
    }
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

pub async fn gas_price_worker<C, S>(
    settlement_client: Arc<dyn SettlementClientTrait<Config = C, StreamType = S>>,
    backend: Arc<MadaraBackend>,
    mut ctx: ServiceContext,
    gas_provider_config: GasPriceProviderConfig,
    _l1_block_metrics: Arc<L1BlockMetrics>,
) -> Result<(), SettlementClientError>
where
    S: Stream<Item = Result<L1toL2MessagingEventData, SettlementClientError>> + Send + 'static,
{
    let mut last_update_timestamp = SystemTime::now();
    let mut interval = tokio::time::interval(gas_provider_config.poll_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    while ctx.run_until_cancelled(interval.tick()).await.is_some() {
        let res = tokio::time::timeout(
            gas_provider_config.poll_interval * 5,
            update_gas_price(Arc::clone(&settlement_client), &gas_provider_config),
        );

        match res.await {
            Ok(Ok(l1_gas_quote)) => {
                last_update_timestamp = SystemTime::now();
                backend.set_last_l1_gas_quote(l1_gas_quote);
                tracing::trace!("Gas prices updated successfully");
            }
            Ok(Err(e)) => {
                tracing::error!("Failed to update gas prices: {:?}", e);
            }
            Err(_) => {
                tracing::warn!("Gas price worker timed out, retrying...");
            }
        }

        if last_update_timestamp.duration_since(SystemTime::UNIX_EPOCH).expect("SystemTime::now() < Unix epoch")
            > gas_provider_config.poll_interval * 10
        {
            return Err(SettlementClientError::GasPrice(format!(
                "Failed to update gas prices for more than 10x poll interval: {:?}",
                gas_provider_config.poll_interval
            )));
        }
    }
    Ok(())
}

pub async fn update_gas_price<C, S>(
    settlement_client: Arc<dyn SettlementClientTrait<Config = C, StreamType = S>>,
    gas_provider_config: &GasPriceProviderConfig,
) -> Result<L1GasQuote, SettlementClientError>
where
    S: Stream<Item = Result<L1toL2MessagingEventData, SettlementClientError>> + Send + 'static,
{
    let mut l1_gas_quote: L1GasQuote = gas_provider_config.into();

    let need_gas_price_from_settlement =
        gas_provider_config.fix_gas_price.is_none() || gas_provider_config.fix_data_gas_price.is_none();
    if need_gas_price_from_settlement {
        let (eth_gas_price, avg_blob_base_fee) = settlement_client.get_gas_prices().await.map_err(|e| {
            SettlementClientError::GasPrice(format!("Failed to get gas prices from settlement client: {}", e))
        })?;
        if gas_provider_config.fix_gas_price.is_none() {
            l1_gas_quote.l1_gas_price = eth_gas_price;
        };
        if gas_provider_config.fix_data_gas_price.is_none() {
            l1_gas_quote.l1_data_gas_price = avg_blob_base_fee;
        };
    }

    let need_oracle = gas_provider_config.fix_strk_per_eth.is_none();
    if need_oracle {
        let (strk_eth_price, decimals) = gas_provider_config
            .oracle_provider
            .as_ref()
            .expect("Oracle is needed if no fix_strk_per_eth is set")
            .fetch_strk_per_eth()
            .await
            .map_err(|e| {
                SettlementClientError::PriceOracle(format!("Failed to fetch STRK/ETH price from oracle: {}", e))
            })?;
        l1_gas_quote.strk_per_eth = (strk_eth_price, decimals);
    }

    Ok(l1_gas_quote)
}

#[cfg(test)]
mod eth_client_gas_price_worker_test {
    use super::*;
    use crate::eth::eth_client_getter_test::{create_ethereum_client, get_anvil_url};
    use tokio::time::Duration;

    #[tokio::test]
    async fn gas_price_update_works() {
        let eth_client = create_ethereum_client(get_anvil_url());
        let gas_price_provider_config = GasPriceProviderConfigBuilder::default()
            .with_fix_strk_per_eth(1.0)
            .with_poll_interval(Duration::from_millis(500))
            .build()
            .expect("Failed to build GasPriceProviderConfig");

        let l1_gas_quote = update_gas_price(Arc::new(eth_client), &gas_price_provider_config)
            .await
            .expect("Failed to update gas prices");

        assert_eq!(l1_gas_quote.l1_gas_price, 948082986);
        assert_eq!(l1_gas_quote.l1_data_gas_price, 1);
    }

    #[tokio::test]
    async fn gas_price_update_when_gas_price_fix_works() {
        let eth_client = create_ethereum_client(get_anvil_url());
        let gas_price_provider_config = GasPriceProviderConfigBuilder::default()
            .with_fix_gas_price(20)
            .with_fix_strk_per_eth(1.0)
            .with_poll_interval(Duration::from_millis(500))
            .build()
            .expect("Failed to build GasPriceProviderConfig");

        let l1_gas_quote = update_gas_price(Arc::new(eth_client), &gas_price_provider_config)
            .await
            .expect("Failed to update gas prices");

        assert_eq!(l1_gas_quote.l1_gas_price, 20);
        assert_eq!(l1_gas_quote.l1_data_gas_price, 1);
    }

    #[tokio::test]
    async fn gas_price_update_when_data_gas_price_fix_works() {
        let eth_client = create_ethereum_client(get_anvil_url());
        let gas_price_provider_config = GasPriceProviderConfigBuilder::default()
            .with_fix_data_gas_price(20)
            .with_fix_strk_per_eth(1.0)
            .with_poll_interval(Duration::from_millis(500))
            .build()
            .expect("Failed to build GasPriceProviderConfig");

        let l1_gas_quote = update_gas_price(Arc::new(eth_client), &gas_price_provider_config)
            .await
            .expect("Failed to update gas prices");

        assert_eq!(l1_gas_quote.l1_gas_price, 948082986);
        assert_eq!(l1_gas_quote.l1_data_gas_price, 20);
    }

    // TODO: this test need a MadaraBackend
    // #[tokio::test]
    // async fn gas_price_worker_when_eth_fee_history_fails_should_fails() {
    //     let mock_server = MockServer::start();
    //     let addr = format!("http://{}", mock_server.address());
    //     let eth_client = create_ethereum_client(addr);
    //     let l1_block_metrics = L1BlockMetrics::register().expect("Failed to register L1 block metrics");
    //
    //     let mock = mock_server.mock(|when, then| {
    //         when.method("POST").path("/").json_body_obj(&serde_json::json!({
    //             "jsonrpc": "2.0",
    //             "method": "eth_feeHistory",
    //             "params": ["0x12c", "0x137368e", []],
    //             "id": 1
    //         }));
    //         then.status(500).json_body_obj(&serde_json::json!({
    //             "jsonrpc": "2.0",
    //             "error": {
    //                 "code": -32000,
    //                 "message": "Internal Server Error"
    //             },
    //             "id": 1
    //         }));
    //     });
    //
    //     mock_server.mock(|when, then| {
    //         when.method("POST")
    //             .path("/")
    //             .json_body_obj(&serde_json::json!({"id":0,"jsonrpc":"2.0","method":"eth_blockNumber"}));
    //         then.status(200).json_body_obj(&serde_json::json!({"jsonrpc":"2.0","id":1,"result":"0x0137368e"}));
    //     });
    //
    //     let gas_price_provider_config = GasPriceProviderConfigBuilder::default()
    //         .with_fix_strk_per_eth(1.0)
    //         .with_poll_interval(Duration::from_millis(500))
    //         .build()
    //         .expect("Failed to build GasPriceProviderConfig");
    //
    //     let timeout_duration = Duration::from_secs(10);
    //
    //     let result = timeout(
    //         timeout_duration,
    //         gas_price_worker::<EthereumClientConfig, EthereumEventStream>(
    //             Arc::new(eth_client),
    //             backend,
    //             ServiceContext::new_for_testing(),
    //             gas_price_provider_config,
    //             Arc::new(l1_block_metrics),
    //         ),
    //     )
    //     .await;
    //
    //     match result {
    //         Ok(Ok(_)) => panic!("Expected gas_price_worker to fail, but it succeeded"),
    //         Ok(Err(err)) => match err {
    //             SettlementClientError::GasPrice(e) => {
    //                 let error_msg = e.to_string();
    //                 let re =
    //                     Regex::new(r"Gas prices have not been updated for \d+ ms\. Last update was at \d+").unwrap();
    //                 assert!(re.is_match(&error_msg), "Error message did not match expected format. Got: {}", error_msg);
    //             }
    //             other => panic!("Expected Other error variant, got: {:?}", other),
    //         },
    //         Err(err) => panic!("gas_price_worker timed out: {err}"),
    //     }
    //
    //     // Verify that the mock was called
    //     mock.assert();
    // }
}
