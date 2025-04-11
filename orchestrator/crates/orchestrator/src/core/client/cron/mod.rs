use crate::core::client::cron::event_bridge::TriggerArns;
use crate::types::jobs::WorkerTriggerType;
use async_trait::async_trait;

pub mod error;
pub mod event_bridge;

/// Trait defining database operations
#[async_trait]
pub trait CronClient: Send + Sync {
    async fn add_cron_target_queue(
        &self,
        trigger_type: &WorkerTriggerType,
        trigger_arns: &TriggerArns,
    ) -> color_eyre::Result<()>;
}
