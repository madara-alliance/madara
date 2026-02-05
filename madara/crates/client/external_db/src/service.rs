//! External database service implementation.

use crate::config::ExternalDbConfig;
use crate::metrics::ExternalDbMetrics;
use crate::writer::ExternalDbWorker;
use mc_db::MadaraBackend;
use mp_utils::service::{MadaraServiceId, PowerOfTwo, Service, ServiceId, ServiceRunner};
use std::sync::Arc;

/// External database service for persisting mempool transactions to MongoDB.
pub struct ExternalDbService {
    config: ExternalDbConfig,
    backend: Arc<MadaraBackend>,
    chain_id: String,
    metrics: Arc<ExternalDbMetrics>,
}

impl ExternalDbService {
    /// Creates a new external database service.
    pub fn new(config: ExternalDbConfig, chain_id: String, backend: Arc<MadaraBackend>) -> anyhow::Result<Self> {
        let config = config.with_chain_id(&chain_id);
        let metrics = Arc::new(ExternalDbMetrics::register());

        Ok(Self { config, backend, chain_id, metrics })
    }

    /// Returns the configuration.
    pub fn config(&self) -> &ExternalDbConfig {
        &self.config
    }
}

#[async_trait::async_trait]
impl Service for ExternalDbService {
    async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        let worker = ExternalDbWorker::new(
            self.config.clone(),
            self.backend.clone(),
            self.chain_id.clone(),
            self.metrics.clone(),
        );

        runner.service_loop(move |ctx| async move { worker.run(ctx).await });

        Ok(())
    }
}

impl ServiceId for ExternalDbService {
    fn svc_id(&self) -> PowerOfTwo {
        MadaraServiceId::ExternalDb.svc_id()
    }
}
