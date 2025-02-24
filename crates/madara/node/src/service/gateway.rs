use crate::cli::GatewayParams;
use mc_db::MadaraBackend;
use mc_rpc::providers::{AddTransactionProvider, AddTransactionProviderGroup};
use mp_utils::service::{MadaraServiceId, Service, ServiceId, ServiceIdProvider, ServiceRunner};
use std::sync::Arc;

#[derive(Clone)]
pub struct GatewayService {
    config: GatewayParams,
    db_backend: Arc<MadaraBackend>,
    add_txs_provider_l2_sync: Arc<dyn AddTransactionProvider>,
    add_txs_provider_mempool: Arc<dyn AddTransactionProvider>,
}

impl GatewayService {
    pub async fn new(
        config: GatewayParams,
        db_backend: Arc<MadaraBackend>,
        add_txs_provider_l2_sync: Arc<dyn AddTransactionProvider>,
        add_txs_provider_mempool: Arc<dyn AddTransactionProvider>,
    ) -> anyhow::Result<Self> {
        Ok(Self { config, db_backend, add_txs_provider_l2_sync, add_txs_provider_mempool })
    }
}

#[async_trait::async_trait]
impl Service for GatewayService {
    async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        let GatewayService { config, db_backend, add_txs_provider_l2_sync, add_txs_provider_mempool } = self.clone();

        runner.service_loop(move |ctx| {
            let add_tx_provider = Arc::new(AddTransactionProviderGroup::new(
                add_txs_provider_l2_sync,
                add_txs_provider_mempool,
                ctx.clone(),
            ));

            mc_gateway_server::service::start_server(
                db_backend,
                add_tx_provider,
                config.feeder_gateway_enable,
                config.gateway_enable,
                config.gateway_external,
                config.gateway_port,
                ctx,
            )
        });
        Ok(())
    }
}

impl ServiceIdProvider for GatewayService {
    #[inline(always)]
    fn id_provider(&self) -> impl ServiceId {
        MadaraServiceId::Gateway
    }
}
