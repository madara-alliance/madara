use clap::Args;

/// CLI arguments for the aws event bridge.
#[derive(Debug, Clone, Args)]
#[group()]
pub struct AWSEventBridgeCliArgs {
    /// Use the AWS Event Bridge client
    #[arg(long)]
    pub aws_event_bridge: bool,

    /// The name of the queue for the event bridge
    #[arg(env = "MADARA_ORCHESTRATOR_EVENT_BRIDGE_TARGET_QUEUE_NAME", long, default_value = Some("madara_orchestrator_worker_trigger_queue"), help = "The name of the SNS queue to send messages to from the event bridge.")]
    pub target_queue_name: Option<String>,
    /// The cron time for the event bridge trigger rule.
    #[arg(env = "MADARA_ORCHESTRATOR_EVENT_BRIDGE_CRON_TIME", long, default_value = Some("60"), help = "The cron time for the event bridge trigger rule. Defaults to 10 seconds.")]
    pub cron_time: Option<String>,

    /// The name of the event bridge trigger rule.
    #[arg(env = "MADARA_ORCHESTRATOR_EVENT_BRIDGE_TRIGGER_RULE_NAME", long, default_value = Some("madara-orchestrator-event-bridge-trigger-rule-name"), help = "The name of the event bridge trigger rule.")]
    pub trigger_rule_name: Option<String>,
}
