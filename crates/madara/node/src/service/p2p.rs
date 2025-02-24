use crate::cli::P2pParams;
use anyhow::Context;
use mc_db::DatabaseService;
use mc_p2p::P2pCommands;
use mp_utils::service::{MadaraServiceId, Service, ServiceId, ServiceIdProvider, ServiceRunner};
use std::time::Duration;

pub struct P2pService {
    enabled: bool,
    // add_transaction_provider: Arc<dyn AddTransactionProvider>,
    p2p: Option<mc_p2p::MadaraP2pBuilder>,
}

impl P2pService {
    pub async fn new(
        config: P2pParams,
        db: &DatabaseService,
        // add_transaction_provider: Arc<dyn AddTransactionProvider>,
    ) -> anyhow::Result<Self> {
        let p2p = if config.p2p {
            let p2p_config = mc_p2p::P2pConfig {
                bootstrap_nodes: db.backend().chain_config().p2p_bootstrap_nodes.clone(),
                port: config.p2p_port,
                status_interval: Duration::from_secs(3),
                identity_file: config.p2p_identity_file,
                save_identity: config.p2p_save_identity,
            };
            let p2p = mc_p2p::MadaraP2pBuilder::new(p2p_config, db.backend().clone() /*add_transaction_provider*/)
                .context("Building p2p service")?;
            Some(p2p)
        } else {
            None
        };

        Ok(Self { p2p, enabled: config.p2p })
    }

    pub fn commands(&mut self) -> Option<P2pCommands> {
        self.p2p.as_ref().map(|p2p| p2p.commands())
    }
}

#[async_trait::async_trait]
impl Service for P2pService {
    async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        if self.enabled {
            let p2p = self.p2p.take().expect("Service already started");
            runner.service_loop(move |ctx| async move { p2p.build().context("Building p2p service")?.run(ctx).await });
        }
        Ok(())
    }
}

impl ServiceIdProvider for P2pService {
    #[inline(always)]
    fn id_provider(&self) -> impl ServiceId {
        MadaraServiceId::P2P
    }
}
