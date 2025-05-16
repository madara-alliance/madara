use clap::Args;

/// Parameters used to config AWS SNS.
#[derive(Debug, Clone, Args)]
#[group()]
pub struct AWSSNSCliArgs {
    /// Use the AWS SNS client
    #[arg(long)]
    pub aws_sns: bool,

    /// The ARN of the SNS topic.
    #[arg(env = "MADARA_ORCHESTRATOR_AWS_SNS_TOPIC_NAME", long, default_value = Some("arn")
    )]
    pub alert_topic_name: Option<String>,
}
