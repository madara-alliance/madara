//! One proof-index worker for ingestion and idle-lease expiry; independent of Mongo availability.
use mc_db::MadaraBackend;
use mp_utils::service::{MadaraServiceId, PowerOfTwo, Service, ServiceId, ServiceRunner};
use std::{sync::Arc, time::Duration};

/// Also reclaim expired disk leases when the chain is idle.
const CLEANUP_INTERVAL: Duration = Duration::from_secs(60);

pub struct CommittedDataHistoryService(pub Arc<MadaraBackend>);
#[async_trait::async_trait]
impl Service for CommittedDataHistoryService {
    async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        let backend = Arc::clone(&self.0);
        runner.service_loop_with_graceful_shutdown(move |mut ctx| async move {
            let mut ticker = tokio::time::interval(CLEANUP_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                let wake = async {
                    tokio::select! {
                        _ = backend.wait_for_committed_data_cleanup() => {},
                        _ = ticker.tick() => {},
                    }
                };
                if ctx.run_until_cancelled(wake).await.is_none() {
                    return anyhow::Ok(());
                }
                // No notification is registered during a sweep: duplicate triggers are dropped.
                loop {
                    if ctx.is_cancelled() {
                        return anyhow::Ok(());
                    }
                    let db = Arc::clone(&backend);
                    // Await an admitted operation even on shutdown: never detach a DB mutation.
                    match tokio::task::spawn_blocking(move || db.maintain_committed_data_history()).await? {
                        Ok(true) => {}
                        Ok(false) => break,
                        Err(error) => {
                            tracing::error!("Committed-data proof pruning deferred; archives retained: {error:#}");
                            break;
                        }
                    }
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
