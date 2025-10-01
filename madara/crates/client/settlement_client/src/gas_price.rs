use crate::error::SettlementClientError;
use crate::SettlementLayerProvider;
use mc_analytics::register_gauge_metric_instrument;
use mc_db::MadaraBackend;
use mp_block::L1GasQuote;
use mp_convert::FixedPoint;
use mp_oracle::Oracle;
use mp_utils::service::ServiceContext;
use opentelemetry::metrics::Gauge;
use opentelemetry::{global, InstrumentationScope, KeyValue};
use std::sync::Arc;
use std::time::Duration;
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
    pub fix_strk_per_eth: Option<FixedPoint>,
    pub oracle_provider: Option<Arc<dyn Oracle>>,
    pub poll_interval: Duration,
}

impl GasPriceProviderConfig {
    pub fn all_is_fixed(&self) -> bool {
        self.fix_gas_price.is_some() && self.fix_data_gas_price.is_some() && self.fix_strk_per_eth.is_some()
    }

    pub fn settlement_and_data_gas_fixed(&self) -> bool {
        self.fix_gas_price.is_some() && self.fix_data_gas_price.is_some()
    }
}

impl From<&GasPriceProviderConfig> for L1GasQuote {
    fn from(value: &GasPriceProviderConfig) -> Self {
        Self {
            l1_gas_price: value.fix_gas_price.unwrap_or_default(),
            l1_data_gas_price: value.fix_data_gas_price.unwrap_or_default(),
            strk_per_eth: value.fix_strk_per_eth.unwrap_or(FixedPoint::one()),
        }
    }
}

pub struct GasPriceProviderConfigBuilder {
    fix_gas_price: Option<u128>,
    fix_data_gas_price: Option<u128>,
    fix_strk_per_eth: Option<FixedPoint>,
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
        self.fix_strk_per_eth = Some(value.into());
        self
    }

    pub fn set_fix_strk_per_eth(&mut self, value: f64) {
        self.fix_strk_per_eth = Some(value.into());
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
            // TODO: fix this messagee.
            tracing::debug!("Oracle provider must be set if no fix_strk_per_eth is provided. Gas prices may be incorrect when producing blocks.");
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
    pub fn register() -> anyhow::Result<Self> {
        let meter = global::meter_with_scope(
            InstrumentationScope::builder("crates.eth.opentelemetry")
                .with_attributes([KeyValue::new("crate", "eth")])
                .build(),
        );

        let l1_block_number = register_gauge_metric_instrument(
            &meter,
            "l1_block_number".to_string(),
            "Gauge for madara L1 block number".to_string(),
            "".to_string(),
        );

        let l1_gas_price_wei = register_gauge_metric_instrument(
            &meter,
            "l1_gas_price_wei".to_string(),
            "Gauge for madara L1 gas price in wei".to_string(),
            "".to_string(),
        );

        let l1_gas_price_strk = register_gauge_metric_instrument(
            &meter,
            "l1_gas_price_strk".to_string(),
            "Gauge for madara L1 gas price in strk".to_string(),
            "".to_string(),
        );

        Ok(Self { l1_block_number, l1_gas_price_wei, l1_gas_price_strk })
    }
}

pub async fn gas_price_worker(
    settlement_client: Arc<dyn SettlementLayerProvider>,
    backend: Arc<MadaraBackend>,
    mut ctx: ServiceContext,
    gas_provider_config: GasPriceProviderConfig,
    _l1_block_metrics: Arc<L1BlockMetrics>,
) -> Result<(), SettlementClientError> {
    let mut last_update_timestamp = SystemTime::now();
    let mut interval = tokio::time::interval(gas_provider_config.poll_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

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

        if SystemTime::now().duration_since(last_update_timestamp).expect("SystemTime::now() < last_update_timestamp")
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

pub async fn update_gas_price(
    settlement_client: Arc<dyn SettlementLayerProvider>,
    gas_provider_config: &GasPriceProviderConfig,
) -> Result<L1GasQuote, SettlementClientError>
where
{
    let mut l1_gas_quote: L1GasQuote = gas_provider_config.into();

    if !gas_provider_config.settlement_and_data_gas_fixed() {
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

    if gas_provider_config.fix_strk_per_eth.is_none() {
        // If no fixed STRK/ETH price is set, fetch it from the oracle
        let strk_per_eth = gas_provider_config
            .oracle_provider
            .as_ref()
            .expect("Oracle is needed if no fix_strk_per_eth is set") // checked in config builder
            .fetch_strk_per_eth()
            .await
            .map_err(|e| {
                SettlementClientError::PriceOracle(format!("Failed to fetch STRK/ETH price from oracle: {}", e))
            })?;
        l1_gas_quote.strk_per_eth = strk_per_eth;
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
}
