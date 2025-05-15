use clap::Args;

#[derive(Clone, Debug, clap::ValueEnum)]
pub enum EventBridgeType {
    Rule,
    Schedule,
}

/// CLI arguments for the aws event bridge.
#[derive(Debug, Clone, Args)]
#[group(requires_all = ["aws_event_bridge"])]
pub struct AWSEventBridgeCliArgs {
    /// Use the AWS Event Bridge client
    #[arg(long)]
    pub aws_event_bridge: bool,

    /// The type of Event Bridge to use (rule or schedule)
    #[arg(env = "MADARA_ORCHESTRATOR_EVENT_BRIDGE_TYPE", long, value_enum)]
    pub event_bridge_type: Option<EventBridgeType>,

    /// The name of the queue for the event bridge
    #[arg(env = "MADARA_ORCHESTRATOR_EVENT_BRIDGE_TARGET_QUEUE_NAME", long, default_value = Some("worker_trigger_queue"), help = "The name of the SNS queue to send messages to from the event bridge."
    )]
    pub target_queue_name: Option<String>,

    /// The cron time for the event bridge trigger rule.
    #[arg(env = "MADARA_ORCHESTRATOR_EVENT_BRIDGE_CRON_TIME", long, default_value = Some("60"), help = "The cron time for the event bridge trigger rule. Defaults to 10 seconds."
    )]
    pub cron_time: Option<String>,

    /// The name of the event bridge trigger rule.
    #[arg(env = "MADARA_ORCHESTRATOR_EVENT_BRIDGE_TRIGGER_RULE_NAME", long, default_value = Some("worker-trigger"), help = "The name of the event bridge trigger rule."
    )]
    pub trigger_rule_name: Option<String>,

    /// The name of the queue for the event bridge
    #[arg(env = "MADARA_ORCHESTRATOR_EVENT_BRIDGE_TRIGGER_ROLE_NAME", long, default_value = Some("madara-orchestrator-worker-trigger-role"), help = "The name of the Trigger Role to assign to the event bridge"
    )]
    pub trigger_role_name: Option<String>,

    /// The name of the queue for the event bridge
    #[arg(env = "MADARA_ORCHESTRATOR_EVENT_BRIDGE_TRIGGER_POLICY_NAME", long, default_value = Some("madara-orchestrator-worker-trigger-policy"), help = "The name of the Trigger Policy to assign to the event bridge"
    )]
    pub trigger_policy_name: Option<String>,
}
