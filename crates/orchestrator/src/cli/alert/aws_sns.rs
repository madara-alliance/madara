use clap::Args;

/// Parameters used to config AWS SNS.
#[derive(Debug, Clone, Args)]
#[group()]
pub struct AWSSNSCliArgs {
    /// Use the AWS SNS client
    #[arg(long)]
    pub aws_sns: bool,

    /// The ARN of the SNS topic.
    #[arg(env = "MADARA_ORCHESTRATOR_AWS_SNS_ARN", long, default_value = Some("arn:aws:sns:us-east-1:000000000000:madara-orchestrator-arn"))]
    pub sns_arn: Option<String>,
}
