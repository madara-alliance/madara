pub mod otel;
pub mod snos;
pub mod cloud_provider;
pub mod service;
pub mod database;
pub mod prover;
pub mod da;
pub mod settlement;

pub use otel::OTELConfig;
use crate::cli::alert::aws_sns::AWSSNSCliArgs;
use crate::cli::RunCmd;
use crate::cron::event_bridge::EventBridgeType;
use crate::OrchestratorError;

/// StorageArgs - Arguments used to setup storage resources
#[derive(Debug, Clone)]
pub struct StorageArgs {
    pub bucket_name: String,
    pub bucket_location_constraint: Option<String>,
}

/// QueueArgs - Arguments used to setup queue resources
#[derive(Debug, Clone)]
pub struct QueueArgs {
    pub prefix: String,
    pub suffix: String,
    pub queue_base_url: String,
}

/// AlertArgs - Arguments used to set up alert resources
#[derive(Debug, Clone)]
pub struct AlertArgs {
    pub endpoint: String,
}

/// CronArgs - Arguments used to setup cron resources
#[derive(Debug, Clone)]
pub struct CronArgs {
    pub event_bridge_type: EventBridgeType,
    pub target_queue_name: String,
    pub cron_time: String,
    pub trigger_rule_name: String,
    pub trigger_role_name: String,
    pub trigger_policy_name: String,
}

impl TryFrom<RunCmd> for StorageArgs {
    type Error = OrchestratorError;
    fn try_from(run_cmd: RunCmd) -> Result<Self, Self::Error> {
        Ok(Self {
            bucket_name: format!("{}-{}", run_cmd.aws_config_args.aws_prefix, run_cmd.aws_s3_args.bucket_name.unwrap()),
            bucket_location_constraint: run_cmd.aws_s3_args.bucket_location_constraint,
        })
    }
}


impl TryFrom<AWSSNSCliArgs> for AlertArgs {

    type Error = OrchestratorError;
    fn try_from(args: AWSSNSCliArgs) -> Result<Self, Self::Error> {
        Ok(Self {
            endpoint: args.sns_arn.ok_or_else(|| OrchestratorError::ConfigError("SNS ARN not found".to_string()))?,
        })
    }
}

impl TryFrom<RunCmd> for QueueArgs {
    type Error = OrchestratorError;
    fn try_from(run_cmd: RunCmd) -> Result<Self, Self::Error> {
        let prefix = format!("{}-{}", run_cmd.aws_config_args.aws_prefix, run_cmd.aws_sqs_args.sqs_prefix.ok_or_else(|| OrchestratorError::SetupCommandError("SQS prefix is required".to_string()))?,);
        Ok(Self {
            queue_base_url: run_cmd.aws_sqs_args.queue_base_url.ok_or_else(|| OrchestratorError::SetupCommandError("Queue base URL is required".to_string()))?,
            prefix,
            suffix: run_cmd.aws_sqs_args.sqs_suffix.ok_or_else(|| OrchestratorError::SetupCommandError("SQS suffix is required".to_string()))?,
        })
    }
}
