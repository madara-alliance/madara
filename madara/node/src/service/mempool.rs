use crate::cli::RunCmd;
use mc_db::MadaraBackend;
use mc_mempool::{ExternalOutboxConfig, Mempool, MempoolConfig};
use mp_utils::service::{MadaraServiceId, PowerOfTwo, Service, ServiceId, ServiceRunner};
use std::sync::Arc;

pub struct MempoolService {
    mempool: Arc<Mempool>,
}

impl MempoolService {
    pub fn new(run_cmd: &RunCmd, backend: Arc<MadaraBackend>) -> Self {
        let save_to_db = !run_cmd.validator_params.no_mempool_saving;
        let load_from_db = run_cmd.validator_params.mempool_reload_from_db.unwrap_or(save_to_db);
        let external_outbox = if run_cmd.external_db_params.is_enabled() {
            ExternalOutboxConfig::enabled(run_cmd.external_db_params.external_db_strict_outbox)
        } else {
            ExternalOutboxConfig::default()
        };
        Self {
            mempool: Arc::new(Mempool::new(
                Arc::clone(&backend),
                MempoolConfig { save_to_db, load_from_db, external_outbox },
            )),
        }
    }

    pub fn mempool(&self) -> Arc<Mempool> {
        self.mempool.clone()
    }
}

#[async_trait::async_trait]
impl Service for MempoolService {
    async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        let mempool = self.mempool.clone();
        runner.service_loop(move |ctx| async move { mempool.run_mempool_task(ctx).await });

        Ok(())
    }
}

impl ServiceId for MempoolService {
    #[inline(always)]
    fn svc_id(&self) -> PowerOfTwo {
        MadaraServiceId::Mempool.svc_id()
    }
}
