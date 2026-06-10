use crate::cli::RunCmd;
use mc_db::MadaraBackend;
use mc_mempool::{ExternalOutboxConfig, Mempool, MempoolConfig};
use mp_utils::service::{MadaraServiceId, PowerOfTwo, Service, ServiceId, ServiceRunner};
use std::sync::Arc;

pub struct MempoolService {
    mempool: Arc<Mempool>,
    run_pool: bool,
}

impl MempoolService {
    pub fn new(run_cmd: &RunCmd, backend: Arc<MadaraBackend>) -> Self {
        let external_outbox = if run_cmd.should_run_external_db() {
            ExternalOutboxConfig::enabled(run_cmd.external_db_params.external_db_strict_outbox)
        } else {
            ExternalOutboxConfig::default()
        };
        Self {
            mempool: Arc::new(Mempool::new(
                Arc::clone(&backend),
                MempoolConfig { save_to_db: run_cmd.should_save_mempool_to_db(), external_outbox },
            )),
            run_pool: run_cmd.should_run_mempool(),
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
        let run_pool = self.run_pool;
        runner.service_loop(move |ctx| {
            let mempool = mempool.clone();
            async move {
                if run_pool {
                    mempool.run_mempool_task(ctx).await
                } else {
                    mempool.run_status_task(ctx).await
                }
            }
        });

        Ok(())
    }
}

impl ServiceId for MempoolService {
    #[inline(always)]
    fn svc_id(&self) -> PowerOfTwo {
        MadaraServiceId::Mempool.svc_id()
    }
}
