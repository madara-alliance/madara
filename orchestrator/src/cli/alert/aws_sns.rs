use clap::Args;

/// Parameters used to config AWS SNS.
#[derive(Debug, Clone, Args)]
#[group()]
pub struct AWSSNSCliArgs {
    /// Use the AWS SNS client
    #[arg(long)]
    pub aws_sns: bool,

    /// The ARN / Name of the SNS topic. it can have either name or ARN string
    /// ARN: arn:aws:sns:region:accountID:name
    /// Name: name
    #[arg(env = "MADARA_ORCHESTRATOR_AWS_SNS_TOPIC_IDENTIFIER", long, default_value = Some("alerts"))]
    pub topic_identifier: Option<String>,
}
