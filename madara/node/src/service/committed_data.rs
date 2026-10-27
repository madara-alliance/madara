//! Independently scheduled proof-index maintenance; never depends on Mongo availability.
use mc_db::MadaraBackend;
use mp_utils::service::{MadaraServiceId, PowerOfTwo, Service, ServiceId, ServiceRunner};
use std::{sync::Arc, time::Duration};

pub struct CommittedDataHistoryService(pub Arc<MadaraBackend>);
#[async_trait::async_trait]
impl Service for CommittedDataHistoryService {
    async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        let backend = Arc::clone(&self.0);
        runner.service_loop_with_graceful_shutdown(move |mut ctx| async move {
            loop {
                if ctx.is_cancelled() {
                    return anyhow::Ok(());
                }
                let db = Arc::clone(&backend);
                // Await an admitted operation even on shutdown: never detach a DB mutation.
                let more = tokio::task::spawn_blocking(move || db.maintain_committed_data_history()).await??;
                if !more && ctx.run_until_cancelled(tokio::time::sleep(Duration::from_secs(5))).await.is_none() {
                    return anyhow::Ok(());
                }
            }
        });
        Ok(())
    }
}
impl ServiceId for CommittedDataHistoryService {
    fn svc_id(&self) -> PowerOfTwo {
        MadaraServiceId::CommittedDataHistory.svc_id()
    }
}
