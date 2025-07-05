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

    /// The cron time for the event bridge trigger rule.
    #[arg(
        env = "MADARA_ORCHESTRATOR_EVENT_BRIDGE_INTERVAL_SECONDS",
        long,
        default_value = "60",
        help = "The interval in seconds for the event bridge trigger rule. Defaults to 60 seconds."
    )]
    pub interval_seconds: Option<u64>,
}
