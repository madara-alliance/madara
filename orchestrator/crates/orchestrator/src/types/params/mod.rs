pub mod cloud_provider;
pub mod da;
pub mod database;
pub mod otel;
pub mod prover;
pub mod service;
pub mod settlement;
pub mod snos;

use crate::cli::alert::aws_sns::AWSSNSCliArgs;
use crate::cli::cron::event_bridge::EventBridgeType;
use crate::cli::{RunCmd, SetupCmd};
use crate::OrchestratorError;
pub use otel::OTELConfig;

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

// REVIEW: 10 : For storage, queue, alert, cron we are taking run_cmd and directly using aws variables to access values,
// should we not validate that aws: bool is true ? and accordingly decide ? 
// overall I think we can generalise these even more to have other providers integrated very easily! 
// e.g : I like how we are handling `match (run_cmd.sharp_args.sharp, run_cmd.atlantic_args.atlantic) {`



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
                    .ok_or(OrchestratorError::SetupCommandError("Missing bucket name".to_string()))?
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


// REVIEW: 9 : Why are we not implementing TryFrom on RunCmd for AlertArgs ?
impl TryFrom<AWSSNSCliArgs> for AlertArgs {
    type Error = OrchestratorError;
    fn try_from(args: AWSSNSCliArgs) -> Result<Self, Self::Error> {
        Ok(Self {
            endpoint: args.sns_arn.ok_or(OrchestratorError::SetupCommandError("SNS ARN not found".to_string()))?,
        })
    }
}

impl TryFrom<SetupCmd> for AlertArgs {
    type Error = OrchestratorError;
    fn try_from(setup_cmd: SetupCmd) -> Result<Self, Self::Error> {
        let sns_arn = setup_cmd
            .aws_sns_args
            .sns_arn
            .ok_or(OrchestratorError::SetupCommandError("SNS ARN not found".to_string()))?;
        // let prefix = setup_cmd.aws_config_args.aws_prefix.clone();
        // let mut parts: Vec<&str> = sns_arn.split(':').collect();
        //
        // if let Some(last) = parts.last_mut() {
        //     // Replace the last part with prefix + "-" + original last part
        //     *last = format!("{}-{}", prefix.clone(), last).as_str();
        // }
        // let result_sns_arn = parts.join(":");
        Ok(Self { endpoint: sns_arn })
    }
}

impl TryFrom<RunCmd> for QueueArgs {
    type Error = OrchestratorError;
    fn try_from(run_cmd: RunCmd) -> Result<Self, Self::Error> {
        let prefix = format!(
            "{}-{}",
            run_cmd.aws_config_args.aws_prefix,
            run_cmd
                .aws_sqs_args
                .sqs_prefix
                .ok_or(OrchestratorError::SetupCommandError("SQS prefix is required".to_string()))?,
        );
        Ok(Self {
            queue_base_url: run_cmd
                .aws_sqs_args
                .queue_base_url
                .ok_or(OrchestratorError::SetupCommandError("Queue base URL is required".to_string()))?,
            prefix,
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
        let prefix = format!(
            "{}-{}",
            setup_cmd.aws_config_args.aws_prefix,
            setup_cmd
                .aws_sqs_args
                .sqs_prefix
                .ok_or(OrchestratorError::SetupCommandError("SQS prefix is required".to_string()))?,
        );
        Ok(Self {
            queue_base_url: setup_cmd
                .aws_sqs_args
                .queue_base_url
                .ok_or(OrchestratorError::SetupCommandError("Queue base URL is required".to_string()))?,
            prefix,
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
        Ok(Self {
            event_bridge_type: setup_cmd
                .aws_event_bridge_args
                .event_bridge_type
                .clone()
                .ok_or(OrchestratorError::SetupCommandError("Event Bridge type is required".to_string()))?,
            target_queue_name: setup_cmd
                .aws_event_bridge_args
                .target_queue_name
                .clone()
                .ok_or(OrchestratorError::SetupCommandError("Target queue name is required".to_string()))?,
            cron_time: setup_cmd
                .aws_event_bridge_args
                .cron_time
                .clone()
                .ok_or(OrchestratorError::SetupCommandError("Cron time is required".to_string()))?,
            trigger_rule_name: setup_cmd
                .aws_event_bridge_args
                .trigger_rule_name
                .clone()
                .ok_or(OrchestratorError::SetupCommandError("Trigger rule name is required".to_string()))?,
            trigger_role_name: setup_cmd
                .aws_event_bridge_args
                .trigger_role_name
                .clone()
                .ok_or(OrchestratorError::SetupCommandError("Trigger role name is required".to_string()))?,
            trigger_policy_name: setup_cmd
                .aws_event_bridge_args
                .trigger_policy_name
                .clone()
                .ok_or(OrchestratorError::SetupCommandError("Trigger policy name is required".to_string()))?,
        })
    }
}
