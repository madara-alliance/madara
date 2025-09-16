use crate::cli::RunCmd;
use mc_db::MadaraBackend;
use mc_mempool::{Mempool, MempoolConfig};
use mp_utils::service::{MadaraServiceId, PowerOfTwo, Service, ServiceId, ServiceRunner};
use std::sync::Arc;

pub struct MempoolService {
    mempool: Arc<Mempool>,
}

impl MempoolService {
    pub fn new(run_cmd: &RunCmd, backend: Arc<MadaraBackend>) -> Self {
        Self {
            mempool: Arc::new(Mempool::new(
                Arc::clone(&backend),
                MempoolConfig { save_to_db: !run_cmd.validator_params.no_mempool_saving },
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
