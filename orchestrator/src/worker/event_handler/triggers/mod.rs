pub(crate) mod batching;
pub(crate) mod data_submission;
pub(crate) mod proof_registration;
pub(crate) mod proving;
pub(crate) mod snos;
pub(crate) mod update_state;

use crate::core::config::Config;
use async_trait::async_trait;
use std::sync::Arc;

/// Result of processing a single proving job
enum ProcessingResult {
    Created,
    Skipped,
    Failed,
}

#[async_trait]
pub trait JobTrigger: Send + Sync {
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()>;
}
