use clap::Args;
use serde::Serialize;

/// Parameters used to config AWS.
#[derive(Debug, Clone, Args, Serialize)]
pub struct AWSConfigCliArgs {
    /// Use this flag to enable AWS provider.
    #[arg(long)]
    pub aws: bool,
    /// The prefix value.
    /// And added to the start of each resource name if available
    #[arg(env = "MADARA_ORCHESTRATOR_AWS_PREFIX", long, default_value = None)]
    pub aws_prefix: Option<String>,
}
