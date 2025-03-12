use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::error::Result;

/// Trigger types for scheduled jobs
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TriggerType {
    /// Snos worker trigger
    Snos,

    /// Proving worker trigger
    Proving,

    /// Proof registration worker trigger
    ProofRegistration,

    /// Data submission worker trigger
    DataSubmission,

    /// State update worker trigger
    UpdateState,

    /// Custom trigger type
    Custom(String),
}

/// Schedule type for cron jobs
#[derive(Debug, Clone)]
pub enum ScheduleType {
    /// Rate-based schedule (e.g., every 5 minutes)
    Rate(Duration),

    /// Cron-based schedule
    Cron(String),

    /// One-time schedule
    OneTime(chrono::DateTime<chrono::Utc>),
}

/// Target configuration for a scheduled job
#[derive(Debug, Clone)]
pub struct TargetConfig {
    /// Target identifier
    pub id: String,

    /// Target ARN or URL
    pub target_arn: String,

    /// Role ARN (if applicable)
    pub role_arn: Option<String>,

    /// Input to pass to the target
    pub input: Option<String>,
}

/// Trait defining cron operations
#[async_trait]
pub trait CronClient: Send + Sync {
    /// Initialize the cron client
    async fn init(&self) -> Result<()>;

    /// Create a schedule with a specific type
    async fn create_schedule(&self, name: &str, schedule_type: ScheduleType, enabled: bool) -> Result<String>;

    /// Add a target to a schedule
    async fn add_target_to_schedule(&self, schedule_name: &str, target: TargetConfig) -> Result<()>;

    /// Delete a schedule
    async fn delete_schedule(&self, name: &str) -> Result<()>;

    /// Enable a schedule
    async fn enable_schedule(&self, name: &str) -> Result<()>;

    /// Disable a schedule
    async fn disable_schedule(&self, name: &str) -> Result<()>;

    /// List schedules matching a pattern
    async fn list_schedules(&self, name_prefix: Option<&str>) -> Result<Vec<String>>;

    /// Get a schedule's details
    async fn get_schedule(&self, name: &str) -> Result<Option<(ScheduleType, Vec<TargetConfig>, bool)>>;

    /// Create a schedule for a worker trigger
    async fn create_worker_trigger(
        &self,
        trigger_type: TriggerType,
        schedule_type: ScheduleType,
        target: TargetConfig,
    ) -> Result<String> {
        let name = format!("worker-trigger-{}", trigger_type_to_string(&trigger_type));
        let schedule_id = self.create_schedule(&name, schedule_type, true).await?;
        self.add_target_to_schedule(&name, target).await?;
        Ok(schedule_id)
    }
}

/// Convert TriggerType to string
fn trigger_type_to_string(trigger_type: &TriggerType) -> String {
    match trigger_type {
        TriggerType::Snos => "snos".to_string(),
        TriggerType::Proving => "proving".to_string(),
        TriggerType::ProofRegistration => "proof-registration".to_string(),
        TriggerType::DataSubmission => "data-submission".to_string(),
        TriggerType::UpdateState => "update-state".to_string(),
        TriggerType::Custom(name) => name.clone(),
    }
}
