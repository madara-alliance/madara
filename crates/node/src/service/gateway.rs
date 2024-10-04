use crate::cli::GatewayParams;
use mc_db::{DatabaseService, MadaraBackend};
use mc_rpc::providers::AddTransactionProvider;
use mp_block::H160;
use mp_utils::service::Service;
use starknet_core::types::Felt;
use std::sync::Arc;
use tokio::task::JoinSet;

#[derive(Clone)]
pub struct GatewayService {
    db_backend: Arc<MadaraBackend>,
    add_transaction_provider: Arc<dyn AddTransactionProvider>,
    feeder_gateway_enable: bool,
    gateway_enable: bool,
    gateway_external: bool,
    gateway_port: u16,
    eth_core_contract_address: H160,
    eth_gps_statement_verifier: H160,
    public_key: Felt,
}

impl GatewayService {
    pub async fn new(
        config: &GatewayParams,
        db: &DatabaseService,
        add_transaction_provider: Arc<dyn AddTransactionProvider>,
        eth_core_contract_address: H160,
        eth_gps_statement_verifier: H160,
        public_key: Felt,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            db_backend: Arc::clone(db.backend()),
            add_transaction_provider,
            feeder_gateway_enable: config.feeder_gateway_enable,
            gateway_enable: config.gateway_enable,
            gateway_external: config.gateway_external,
            gateway_port: config.gateway_port,
            eth_core_contract_address,
            eth_gps_statement_verifier,
            public_key,
        })
    }
}

#[async_trait::async_trait]
impl Service for GatewayService {
    async fn start(&mut self, join_set: &mut JoinSet<anyhow::Result<()>>) -> anyhow::Result<()> {
        if self.feeder_gateway_enable || self.gateway_enable {
            let GatewayService {
                db_backend,
                add_transaction_provider,
                feeder_gateway_enable,
                gateway_enable,
                gateway_external,
                gateway_port,
                eth_core_contract_address,
                eth_gps_statement_verifier,
                public_key,
            } = self.clone();

            join_set.spawn(async move {
                mc_gateway::server::service::start_server(
                    db_backend,
                    add_transaction_provider,
                    feeder_gateway_enable,
                    gateway_enable,
                    gateway_external,
                    gateway_port,
                    eth_core_contract_address,
                    eth_gps_statement_verifier,
                    public_key,
                )
                .await
            });
        }
        Ok(())
    }
}
