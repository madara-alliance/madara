pub mod cloud_provider;
pub mod da;
pub mod database;
pub mod otel;
pub mod prover;
pub mod service;
pub mod settlement;
pub mod snos;

use crate::cli::cron::event_bridge::EventBridgeType;
use crate::cli::{RunCmd, SetupCmd};
use crate::OrchestratorError;
pub use otel::OTELConfig;
use tracing::info;

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
}

/// AlertArgs - Arguments used to set up alert resources
#[derive(Debug, Clone)]
pub struct AlertArgs {
    pub alert_topic_name: String,
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

/// Miscellaneous arguments
#[derive(Debug, Clone)]
pub struct MiscellaneousArgs {
    pub poll_interval: u64,
    pub timeout: u64,
}

/// NOTE: The following implementations are used to convert the command line arguments
/// to the respective argument structs. These implementations are used to validate the command line arguments
/// and convert them to the respective argument structs.
/// Since we have only one Cloud Provider (AWS) for now, we are not using the provider-based implementation.
/// e.g : I like how we are handling `match (run_cmd.sharp_args.sharp, run_cmd.atlantic_args.atlantic)`
impl TryFrom<SetupCmd> for MiscellaneousArgs {
    type Error = OrchestratorError;
    fn try_from(setup_cmd: SetupCmd) -> Result<Self, Self::Error> {
        Ok(Self {
            timeout: setup_cmd
                .timeout
                .ok_or_else(|| OrchestratorError::SetupCommandError("Timeout is required".to_string()))?,
            poll_interval: setup_cmd
                .poll_interval
                .ok_or_else(|| OrchestratorError::SetupCommandError("Poll interval is required".to_string()))?,
        })
    }
}

impl TryFrom<RunCmd> for StorageArgs {
    type Error = OrchestratorError;
    fn try_from(run_cmd: RunCmd) -> Result<Self, Self::Error> {
        Ok(Self {
            bucket_name: format!(
                "{}-{}",
                run_cmd.aws_config_args.aws_prefix,
                run_cmd
                    .aws_s3_args
                    .bucket_name
                    .ok_or(OrchestratorError::SetupCommandError("Bucket name Not found".to_string()))?
            ),
            bucket_location_constraint: run_cmd.aws_s3_args.bucket_location_constraint,
        })
    }
}

impl TryFrom<SetupCmd> for StorageArgs {
    type Error = OrchestratorError;
    fn try_from(setup_cmd: SetupCmd) -> Result<Self, Self::Error> {
        Ok(Self {
            bucket_name: format!(
                "{}-{}",
                setup_cmd.aws_config_args.aws_prefix,
                setup_cmd
                    .aws_s3_args
                    .bucket_name
                    .ok_or(OrchestratorError::SetupCommandError("Missing bucket name".to_string()))?
            ),
            bucket_location_constraint: setup_cmd.aws_s3_args.bucket_location_constraint,
        })
    }
}

impl TryFrom<SetupCmd> for AlertArgs {
    type Error = OrchestratorError;
    fn try_from(setup_cmd: SetupCmd) -> Result<Self, Self::Error> {
        let topic = setup_cmd
            .aws_sns_args
            .alert_topic_name
            .ok_or(OrchestratorError::SetupCommandError("SNS ARN not found".to_string()))?;
        let alert_topic_name = format!("{}_{}", setup_cmd.aws_config_args.aws_prefix, topic);
        info!("SNS TOPIC: {}", alert_topic_name);
        Ok(Self { alert_topic_name })
    }
}

impl TryFrom<RunCmd> for AlertArgs {
    type Error = OrchestratorError;
    fn try_from(run_cmd: RunCmd) -> Result<Self, Self::Error> {
        let topic = run_cmd
            .aws_sns_args
            .alert_topic_name
            .ok_or(OrchestratorError::SetupCommandError("SNS ARN not found".to_string()))?;
        let alert_topic_name = format!("{}_{}", run_cmd.aws_config_args.aws_prefix, topic);
        info!("SNS TOPIC: {}", alert_topic_name);
        Ok(Self { alert_topic_name })
    }
}

impl TryFrom<RunCmd> for QueueArgs {
    type Error = OrchestratorError;
    fn try_from(run_cmd: RunCmd) -> Result<Self, Self::Error> {
        Ok(Self {
            prefix: run_cmd.aws_config_args.aws_prefix.clone(),
            suffix: run_cmd
                .aws_sqs_args
                .sqs_suffix
                .ok_or(OrchestratorError::SetupCommandError("SQS suffix is required".to_string()))?,
        })
    }
}

impl TryFrom<SetupCmd> for QueueArgs {
    type Error = OrchestratorError;
    fn try_from(setup_cmd: SetupCmd) -> Result<Self, Self::Error> {
        Ok(Self {
            prefix: setup_cmd.aws_config_args.aws_prefix.clone(),
            suffix: setup_cmd
                .aws_sqs_args
                .sqs_suffix
                .ok_or(OrchestratorError::SetupCommandError("SQS suffix is required".to_string()))?,
        })
    }
}

impl TryFrom<SetupCmd> for CronArgs {
    type Error = OrchestratorError;
    fn try_from(setup_cmd: SetupCmd) -> Result<Self, Self::Error> {
        let target_queue_name = format!(
            "{}_{}",
            setup_cmd.aws_config_args.aws_prefix,
            setup_cmd
                .aws_event_bridge_args
                .target_queue_name
                .ok_or(OrchestratorError::SetupCommandError("SQS prefix is required".to_string()))?,
        );
        let trigger_rule_name = format!(
            "{}_{}",
            setup_cmd.aws_config_args.aws_prefix,
            setup_cmd
                .aws_event_bridge_args
                .trigger_rule_name
                .ok_or(OrchestratorError::SetupCommandError("Trigger rule name is required".to_string()))?,
        );
        Ok(Self {
            event_bridge_type: setup_cmd
                .aws_event_bridge_args
                .event_bridge_type
                .ok_or(OrchestratorError::SetupCommandError("Event Bridge type is required".to_string()))?,
            target_queue_name,
            cron_time: setup_cmd
                .aws_event_bridge_args
                .cron_time
                .ok_or(OrchestratorError::SetupCommandError("Cron time is required".to_string()))?,
            trigger_rule_name,
            trigger_role_name: setup_cmd
                .aws_event_bridge_args
                .trigger_role_name
                .ok_or(OrchestratorError::SetupCommandError("Trigger role name is required".to_string()))?,
            trigger_policy_name: setup_cmd
                .aws_event_bridge_args
                .trigger_policy_name
                .ok_or(OrchestratorError::SetupCommandError("Trigger policy name is required".to_string()))?,
        })
    }
}
