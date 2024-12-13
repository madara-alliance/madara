use crate::cli::P2pParams;
use anyhow::Context;
use mc_db::{DatabaseService, MadaraBackend};
use mc_rpc::providers::AddTransactionProvider;
use mp_utils::service::Service;
use std::{sync::Arc, time::Duration};
use tokio::task::JoinSet;

#[derive(Clone)]
pub struct P2pService {
    config: P2pParams,
    db_backend: Arc<MadaraBackend>,
    add_transaction_provider: Arc<dyn AddTransactionProvider>,
}

impl P2pService {
    pub async fn new(
        config: P2pParams,
        db: &DatabaseService,
        add_transaction_provider: Arc<dyn AddTransactionProvider>,
    ) -> anyhow::Result<Self> {
        Ok(Self { config, db_backend: Arc::clone(db.backend()), add_transaction_provider })
    }
}

#[async_trait::async_trait]
impl Service for P2pService {
    async fn start(&mut self, join_set: &mut JoinSet<anyhow::Result<()>>) -> anyhow::Result<()> {
        if self.config.p2p {
            let P2pService { db_backend, add_transaction_provider, config } = self.clone();

            let config = mc_p2p::P2pConfig {
                bootstrap_nodes: self.db_backend.chain_config().p2p_bootstrap_nodes.clone(),
                port: config.p2p_port,
                status_interval: Duration::from_secs(3),
                identity_file: config.p2p_identity_file,
                save_identity: config.p2p_save_identity,
            };
            let mut p2p =
                mc_p2p::MadaraP2p::new(config, db_backend, add_transaction_provider).context("Creating p2p service")?;
            join_set.spawn(async move {
                p2p.run().await?;
                Ok(())
            });
        }
        Ok(())
    }
}
