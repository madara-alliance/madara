use async_trait::async_trait;
use serde::Serialize;
use std::collections::HashMap;

use crate::error::Result;

/// Schedule type
#[derive(Debug, Clone)]
pub enum ScheduleType {
    /// One-time schedule
    OneTime(chrono::DateTime<chrono::Utc>),

    /// Rate-based schedule (e.g., every 5 minutes)
    Rate(String),

    /// Cron-based schedule
    Cron(String),
}

/// Target for a scheduled task
#[derive(Debug, Clone)]
pub struct ScheduleTarget {
    /// Target ARN
    pub arn: String,

    /// Role ARN for execution
    pub role_arn: String,

    /// Input for the target
    pub input: Option<String>,
}

/// Trait defining scheduler operations
#[async_trait]
pub trait SchedulerClient: Send + Sync {
    /// Initialize the scheduler client
    async fn init(&self) -> Result<()>;

    /// Create a schedule
    async fn create_schedule(
        &self,
        name: &str,
        schedule_type: ScheduleType,
        target: ScheduleTarget,
        description: Option<&str>,
        enabled: bool,
    ) -> Result<String>;

    /// Create a schedule with structured input
    async fn create_schedule_with_structured_input<T>(
        &self,
        name: &str,
        schedule_type: ScheduleType,
        target_arn: &str,
        role_arn: &str,
        input: &T,
        description: Option<&str>,
        enabled: bool,
    ) -> Result<String>
    where
        T: Serialize + Send + Sync;

    /// Update a schedule
    async fn update_schedule(
        &self,
        name: &str,
        schedule_type: Option<ScheduleType>,
        target: Option<ScheduleTarget>,
        description: Option<&str>,
        enabled: Option<bool>,
    ) -> Result<()>;

    /// Delete a schedule
    async fn delete_schedule(&self, name: &str) -> Result<()>;

    /// Get schedule details
    async fn get_schedule(&self, name: &str) -> Result<(ScheduleType, ScheduleTarget, bool)>;

    /// List schedules
    async fn list_schedules(&self) -> Result<Vec<String>>;

    /// Enable a schedule
    async fn enable_schedule(&self, name: &str) -> Result<()>;

    /// Disable a schedule
    async fn disable_schedule(&self, name: &str) -> Result<()>;
}
