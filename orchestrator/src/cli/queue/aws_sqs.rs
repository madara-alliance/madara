use clap::Args;

/// Parameters used to config AWS SQS.
#[derive(Debug, Clone, Args)]
#[group(requires_all = ["sqs_prefix", "sqs_suffix"])]
pub struct AWSSQSCliArgs {
    /// Use the AWS sqs client
    #[arg(long)]
    pub aws_sqs: bool,

    /// The ARN / Name of the queue.
    /// ARN: arn:aws:sqs:region:accountID:name
    /// {} will be replaced by Queue Type, when independent setup for each queue.
    /// i.e for WorkerTrigger queue : mo_worker_trigger_queue
    #[arg(env = "MADARA_ORCHESTRATOR_AWS_SQS_QUEUE_IDENTIFIER", long, default_value = Some("mo_{}_queue"))]
    pub queue_identifier: Option<String>,
}
