use crate::cli::GatewayParams;
use mc_db::{DatabaseService, MadaraBackend};
use mc_rpc::providers::AddTransactionProvider;
use mp_utils::service::{MadaraService, Service, ServiceRunner};
use std::sync::Arc;

#[derive(Clone)]
pub struct GatewayService {
    config: GatewayParams,
    db_backend: Arc<MadaraBackend>,
    add_transaction_provider: Arc<dyn AddTransactionProvider>,
}

impl GatewayService {
    pub async fn new(
        config: GatewayParams,
        db: &DatabaseService,
        add_transaction_provider: Arc<dyn AddTransactionProvider>,
    ) -> anyhow::Result<Self> {
        Ok(Self { config, db_backend: Arc::clone(db.backend()), add_transaction_provider })
    }
}

#[async_trait::async_trait]
impl Service for GatewayService {
    async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        let GatewayService { db_backend, add_transaction_provider, config } = self.clone();

        runner.service_loop(move |ctx| {
            mc_gateway_server::service::start_server(
                db_backend,
                add_transaction_provider,
                config.feeder_gateway_enable,
                config.gateway_enable,
                config.gateway_external,
                config.gateway_port,
                ctx,
            )
        });
        Ok(())
    }

    fn id(&self) -> MadaraService {
        MadaraService::Gateway
    }
}
