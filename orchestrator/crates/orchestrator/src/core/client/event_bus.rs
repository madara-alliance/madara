use async_trait::async_trait;
use serde::Serialize;
use std::collections::HashMap;

use crate::error::Result;

/// Event detail
#[derive(Debug, Clone)]
pub struct Event {
    /// Source of the event
    pub source: String,

    /// Event type
    pub event_type: String,

    /// Event detail (JSON serializable)
    pub detail: serde_json::Value,

    /// Event resources
    pub resources: Vec<String>,

    /// Event time
    pub time: Option<chrono::DateTime<chrono::Utc>>,
}

/// Rule target
#[derive(Debug, Clone)]
pub struct RuleTarget {
    /// Target ID
    pub id: String,

    /// Target ARN
    pub arn: String,

    /// Input transformer
    pub input_path: Option<String>,

    /// Role ARN for execution
    pub role_arn: Option<String>,
}

/// Trait defining event bus operations
#[async_trait]
pub trait EventBusClient: Send + Sync {
    /// Initialize the event bus client
    async fn init(&self) -> Result<()>;

    /// Create an event bus
    async fn create_event_bus(&self, name: &str) -> Result<String>;

    /// Delete an event bus
    async fn delete_event_bus(&self, name: &str) -> Result<()>;

    /// List event buses
    async fn list_event_buses(&self) -> Result<Vec<String>>;

    /// Put events on the event bus
    async fn put_events(&self, events: &[Event]) -> Result<Vec<String>>;

    /// Put a single event on the event bus
    async fn put_event(&self, event: &Event) -> Result<String>;

    /// Put a structured event on the event bus
    async fn put_structured_event<T>(&self, source: &str, event_type: &str, detail: &T) -> Result<String>
    where
        T: Serialize + Send + Sync;

    /// Create a rule
    async fn create_rule(
        &self,
        event_bus_name: &str,
        rule_name: &str,
        event_pattern: Option<&str>,
        schedule_expression: Option<&str>,
        description: Option<&str>,
        state: &str,
    ) -> Result<String>;

    /// Delete a rule
    async fn delete_rule(&self, event_bus_name: &str, rule_name: &str) -> Result<()>;

    /// List rules for an event bus
    async fn list_rules(&self, event_bus_name: &str) -> Result<Vec<String>>;

    /// Add targets to a rule
    async fn put_targets(&self, event_bus_name: &str, rule_name: &str, targets: &[RuleTarget]) -> Result<()>;

    /// Remove targets from a rule
    async fn remove_targets(&self, event_bus_name: &str, rule_name: &str, target_ids: &[String]) -> Result<()>;
}
