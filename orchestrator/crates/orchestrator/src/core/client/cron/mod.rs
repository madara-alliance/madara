use crate::cli::cron::event_bridge::EventBridgeType;
use crate::core::client::cron::event_bridge::TriggerArns;
use crate::types::jobs::WorkerTriggerType;
use async_trait::async_trait;
use std::time::Duration;

pub mod error;
pub mod event_bridge;

/// Trait defining database operations
#[async_trait]
pub trait CronClient: Send + Sync {
    /// TODO: I dont think this is needed to be here since this will not be a global function
    async fn add_cron_target_queue(
        &self,
        trigger_type: &WorkerTriggerType,
        trigger_arns: &TriggerArns,
        trigger_rule_name: String,
        event_bridge_type: EventBridgeType,
        cron_time: Duration,
    ) -> color_eyre::Result<()>;
}
