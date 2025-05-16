use clap::Args;
use serde::Serialize;

/// Parameters used to config AWS.
#[derive(Debug, Clone, Args, Serialize)]
#[group(requires_all = ["aws_region"])]
pub struct AWSConfigCliArgs {
    /// Use this flag to enable AWS provider.
    #[arg(long)]
    pub aws: bool,

    /// The region.
    #[arg(env = "AWS_REGION", long)]
    pub aws_region: String,

    /// The region.
    #[arg(env = "MADARA_ORCHESTRATOR_AWS_PREFIX", long, default_value = "madara-orchestrator")]
    pub aws_prefix: String,
}
