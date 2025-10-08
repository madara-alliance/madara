use crate::cli::l1::{L1SyncParams, MadaraSettlementLayer};
use anyhow::{bail, Context};
use mc_db::MadaraBackend;
use mc_settlement_client::gas_price::GasPriceProviderConfigBuilder;
use mc_settlement_client::state_update::L1HeadSender;
use mc_settlement_client::sync::SyncWorkerConfig;
use mc_settlement_client::{gas_price::L1BlockMetrics, SettlementClient};
use mc_settlement_client::{L1ClientImpl, L1SyncDisabledClient};
use mp_block::L1GasQuote;
use mp_oracle::pragma::PragmaOracleBuilder;
use mp_utils::service::{MadaraServiceId, PowerOfTwo, Service, ServiceId, ServiceRunner};
use std::sync::Arc;

// Configuration struct to group related parameters
pub struct L1SyncConfig {
    pub l1_core_address: String,
    pub l1_block_metrics: Arc<L1BlockMetrics>,
    pub l1_head_snd: L1HeadSender,
}

pub struct L1SyncService {
    sync_worker_config: Option<SyncWorkerConfig>,
    client: Option<Arc<L1ClientImpl>>,
}

impl L1SyncService {
    pub async fn new(
        config: &L1SyncParams,
        backend: Arc<MadaraBackend>,
        sync_config: L1SyncConfig,
    ) -> anyhow::Result<Self> {
        let mut gas_price_provider_builder = GasPriceProviderConfigBuilder::default();
        if let Some(fix_gas) = config.l1_gas_price {
            gas_price_provider_builder.set_fix_gas_price(fix_gas.into());
        }
        if let Some(fix_blob_gas) = config.blob_gas_price {
            gas_price_provider_builder.set_fix_data_gas_price(fix_blob_gas.into());
        }
        if let Some(strk_per_eth_fix) = config.strk_per_eth {
            gas_price_provider_builder.set_fix_strk_per_eth(strk_per_eth_fix);
        }
        if let Some(ref oracle_url) = config.oracle_url {
            if let Some(ref oracle_api_key) = config.oracle_api_key {
                let oracle = PragmaOracleBuilder::new()
                    .with_api_url(oracle_url.clone())
                    .with_api_key(oracle_api_key.clone())
                    .build();
                gas_price_provider_builder.set_oracle_provider(Arc::new(oracle));
            } else {
                bail!("Only Pragma oracle is supported, please provide the oracle API key");
            }
        }
        let gas_provider_config = gas_price_provider_builder
            .with_poll_interval(config.gas_price_poll)
            .build()
            .context("Building gas price provider config")?;

        if gas_provider_config.all_is_fixed() {
            // safe to unwrap because we checked that all values are set
            let l1_gas_quote = L1GasQuote {
                l1_gas_price: gas_provider_config.fix_gas_price.unwrap(),
                l1_data_gas_price: gas_provider_config.fix_data_gas_price.unwrap(),
                strk_per_eth: gas_provider_config.fix_strk_per_eth.unwrap(),
            };
            backend.set_last_l1_gas_quote(l1_gas_quote);
        }

        if config.l1_sync_disabled {
            return Ok(Self { sync_worker_config: None, client: None });
        }
        let Some(endpoint) = config.l1_endpoint.clone() else {
            tracing::error!("Missing l1_endpoint CLI argument. Either disable L1 sync using `--no-l1-sync` or give an L1 RPC endpoint URL using `--l1-endpoint <url>`.");
            std::process::exit(1);
        };
        let client = match config.settlement_layer {
            MadaraSettlementLayer::Eth => {
                L1ClientImpl::new_ethereum(backend.clone(), endpoint, sync_config.l1_core_address)
                    .await
                    .context("Starting ethereum core contract client")?
            }
            MadaraSettlementLayer::Starknet => {
                L1ClientImpl::new_starknet(backend.clone(), endpoint, sync_config.l1_core_address)
                    .await
                    .context("Starting starknet core contract client")?
            }
        };

        if !gas_provider_config.all_is_fixed() {
            tracing::info!("⏳ Getting initial L1 gas prices");
            // Gas prices are needed before starting the block producer
            let l1_gas_quote =
                mc_settlement_client::gas_price::update_gas_price(client.provider(), &gas_provider_config)
                    .await
                    .context("Getting initial gas prices")?;
            backend.set_last_l1_gas_quote(l1_gas_quote);
        }

        Ok(Self {
            client: Some(client.into()),
            sync_worker_config: Some(SyncWorkerConfig {
                gas_provider_config,
                l1_head_sender: sync_config.l1_head_snd,
                l1_block_metrics: sync_config.l1_block_metrics,
            }),
        })
    }

    pub fn client(&self) -> Arc<dyn SettlementClient> {
        if let Some(client) = self.client.clone() {
            client
        } else {
            Arc::new(L1SyncDisabledClient)
        }
    }
}

#[async_trait::async_trait]
impl Service for L1SyncService {
    async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        if let Some((config, client)) = self.sync_worker_config.take().zip(self.client.clone()) {
            runner.service_loop(move |ctx| client.run_sync_worker(ctx, config));
        } else {
            tracing::debug!("l1 sync is disabled");
        }

        Ok(())
    }
}

impl ServiceId for L1SyncService {
    #[inline(always)]
    fn svc_id(&self) -> PowerOfTwo {
        MadaraServiceId::L1Sync.svc_id()
    }
}
