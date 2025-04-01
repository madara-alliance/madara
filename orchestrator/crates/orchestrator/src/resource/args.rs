use crate::cli::cron::event_bridge::EventBridgeType;

/// StorageArgs - Arguments used to setup storage resources
#[derive(Debug, Clone)]
pub struct StorageArgs {
    pub bucket_name: String,
    pub bucket_location_constraint: Option<String>,
}

/// QueueArgs - Arguments used to setup queue resources
#[derive(Debug, Clone)]
pub struct QueueArgs {
    pub prefix: String,
    pub suffix: String,
    pub queue_base_url: String,
}

/// AlertArgs - Arguments used to set up alert resources
#[derive(Debug, Clone)]
pub struct AlertArgs {
    pub endpoint: String,
}

/// CronArgs - Arguments used to setup cron resources
#[derive(Debug, Clone)]
pub struct CronArgs {
    pub event_bridge_type: EventBridgeType,
    pub target_queue_name: String,
    pub cron_time: String,
    pub trigger_rule_name: String,
    pub trigger_role_name: String,
    pub trigger_policy_name: String,
}
