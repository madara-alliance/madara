use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudProviderConfig {
    pub provider: CloudProvider,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub aws: Option<AWSConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CloudProvider {
    AWS,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AWSConfig {
    #[serde(default = "default_region")]
    pub region: String,

    pub prefix: String,
    pub storage: S3Config,
    pub queue: SQSConfig,
    pub alerts: SNSConfig,
    pub event_bridge: EventBridgeConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S3Config {
    #[serde(default = "default_bucket")]
    pub bucket_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SQSConfig {
    #[serde(default = "default_queue_pattern")]
    pub name_pattern: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SNSConfig {
    #[serde(default = "default_topic")]
    pub topic_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventBridgeConfig {
    #[serde(rename = "type")]
    pub bridge_type: EventBridgeType,

    #[serde(default = "default_interval")]
    pub interval_seconds: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventBridgeType {
    Rule,
    Schedule,
}

fn default_region() -> String {
    "us-east-1".to_string()
}
fn default_bucket() -> String {
    "orchestrator-artifacts".to_string()
}
fn default_queue_pattern() -> String {
    "orchestrator-{job_type}".to_string()
}
fn default_topic() -> String {
    "orchestrator-alerts".to_string()
}
fn default_interval() -> u64 {
    60
}
