use clap::Args;

/// Parameters used to config AWS SQS.
#[derive(Debug, Clone, Args)]
#[group(requires_all = ["queue_base_url"])]
pub struct AWSSQSCliArgs {
    /// Use the AWS sqs client
    #[arg(long)]
    pub aws_sqs: bool,

    /// The prefix of the queue.
    #[arg(env = "MADARA_ORCHESTRATOR_SQS_PREFIX", long, default_value = Some("madara_orchestrator"))]
    pub sqs_prefix: Option<String>,

    /// The suffix of the queue.    
    #[arg(env = "MADARA_ORCHESTRATOR_SQS_SUFFIX", long, default_value = Some("queue"))]
    pub sqs_suffix: Option<String>,

    /// The QUEUE url
    #[arg(env = "MADARA_ORCHESTRATOR_SQS_BASE_QUEUE_URL", long)]
    pub queue_base_url: Option<String>,
}
