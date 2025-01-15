use crate::cli::SyncParams;
use mc_db::MadaraBackend;
use mc_p2p::P2pCommands;
use mp_utils::service::Service;
use std::sync::Arc;
use tokio::task::JoinSet;

#[derive(Clone)]
pub struct Sync2Service {
    db_backend: Arc<MadaraBackend>,
    p2p_commands: Option<mc_p2p::P2pCommands>,
    disabled: bool,
}

impl Sync2Service {
    pub async fn new(_config: &SyncParams, db: &Arc<MadaraBackend>, p2p_commands: P2pCommands) -> anyhow::Result<Self> {
        Ok(Self { db_backend: Arc::clone(db), disabled: false, p2p_commands: Some(p2p_commands) })
    }
}

#[async_trait::async_trait]
impl Service for Sync2Service {
    async fn start(&mut self, join_set: &mut JoinSet<anyhow::Result<()>>) -> anyhow::Result<()> {
        if self.disabled {
            return Ok(());
        }

        let p2p_commands = self.p2p_commands.take().expect("Service already started");
        let args = mc_sync2::P2pPipelineArguments::new(self.db_backend.clone(), p2p_commands);

        join_set.spawn(async move { mc_sync2::forward_sync(args, Default::default()).await });

        Ok(())
    }
}
