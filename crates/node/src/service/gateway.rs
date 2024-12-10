use crate::cli::GatewayParams;
use mc_db::{DatabaseService, MadaraBackend};
use mc_rpc::providers::AddTransactionProvider;
use mp_utils::service::{MadaraService, Service, ServiceContext};
use std::sync::Arc;
use tokio::task::JoinSet;

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
    async fn start(&mut self, join_set: &mut JoinSet<anyhow::Result<()>>, ctx: ServiceContext) -> anyhow::Result<()> {
        if self.config.feeder_gateway_enable || self.config.gateway_enable {
            let GatewayService { db_backend, add_transaction_provider, config } = self.clone();

            join_set.spawn(async move {
                mc_gateway_server::service::start_server(
                    db_backend,
                    add_transaction_provider,
                    config.feeder_gateway_enable,
                    config.gateway_enable,
                    config.gateway_external,
                    config.gateway_port,
                    ctx,
                )
                .await
            });
        }
        Ok(())
    }

    fn id(&self) -> MadaraService {
        MadaraService::Gateway
    }
}
