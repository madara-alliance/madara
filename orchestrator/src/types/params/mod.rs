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
use crate::core::client::queue::sqs::InnerSQS;
use crate::types::queue::QueueType;
use crate::OrchestratorError;
pub use otel::OTELConfig;

#[derive(Debug, Clone)]
pub struct ARN {
    /// AWS partition, usually "aws" (e.g., "aws-us-gov", "aws-cn")
    pub partition: String,
    /// AWS service (e.g., "s3", "sns", "sqs")
    pub service: String,
    /// AWS region (e.g., "us-east-1", can be empty for global services)
    pub region: String,
    /// AWS account ID (12-digit number, can be empty for some resources)
    pub account_id: String,
    /// Resource identifier (e.g., "topic-name", "bucket-name", "queue-name")
    pub resource: String,
}
impl ARN {
    /// Parse an ARN string into its components
    /// Format: arn:partition:service:region:account-id:resource
    pub fn parse(arn_str: &str) -> Result<Self, &'static str> {
        if arn_str.trim().is_empty() {
            return Err("ARN string cannot be empty");
        }

        let parts: Vec<&str> = arn_str.split(':').collect();

        if parts.len() != 6 || parts[0] != "arn" {
            return Err("Invalid ARN format");
        }

        // Check for required non-empty fields
        if parts[1].is_empty() {
            return Err("Partition cannot be empty");
        }

        if parts[2].is_empty() {
            return Err("Service cannot be empty");
        }

        if parts[5].is_empty() {
            return Err("Resource cannot be empty");
        }

        // Note: region and account_id can be empty for some AWS services (like S3)
        if parts[2] != "s3" {
            // region can also be empty for iam.
            if parts[2] != "iam" && parts[3].is_empty() {
                return Err("Region cannot be empty");
            }

            if parts[2] != "iam" && parts[4].is_empty() {
                return Err("Account ID cannot be empty");
            }
        }

        Ok(ARN {
            partition: parts[1].to_string(),
            service: parts[2].to_string(),
            region: parts[3].to_string(),
            account_id: parts[4].to_string(),
            resource: parts[5].to_string(),
        })
    }
}

impl fmt::Display for ARN {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "arn:{}:{}:{}:{}:{}", self.partition, self.service, self.region, self.account_id, self.resource)
    }
}

#[derive(Debug, Clone)]
pub enum AWSResourceIdentifier {
    ARN(ARN),
    Name(String),
}

use std::fmt;
impl fmt::Display for AWSResourceIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AWSResourceIdentifier::ARN(arn) => write!(f, "{}", arn),
            AWSResourceIdentifier::Name(name) => write!(f, "{}", name),
        }
    }
}

/// StorageArgs - Arguments used to setup storage resources
#[derive(Debug, Clone)]
pub struct StorageArgs {
    pub bucket_identifier: AWSResourceIdentifier,
}

impl StorageArgs {
    pub fn format_prefix_and_name(prefix: &str, name: &str) -> String {
        format!("{}-{}", prefix, name)
    }
}

/// QueueArgs - Arguments used to setup queue resources
#[derive(Debug, Clone)]
pub struct QueueArgs {
    pub queue_template_identifier: AWSResourceIdentifier,
}

impl QueueArgs {
    pub fn format_prefix_and_name(prefix: &str, name: &str) -> String {
        format!("{}_{}", prefix, name)
    }
}

/// AlertArgs - Arguments used to set up alert resources
#[derive(Debug, Clone)]
pub struct AlertArgs {
    pub alert_identifier: AWSResourceIdentifier,
}

impl AlertArgs {
    pub fn format_prefix_and_name(prefix: &str, name: &str) -> String {
        format!("{}_{}", prefix, name)
    }
}

/// CronArgs - Arguments used to setup cron resources
#[derive(Debug, Clone)]
pub struct CronArgs {
    pub target_queue_identifier: AWSResourceIdentifier,
    pub event_bridge_type: EventBridgeType,
    pub cron_time: u64,
    pub trigger_rule_template_name: String,
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

impl TryFrom<SetupCmd> for StorageArgs {
    type Error = OrchestratorError;
    fn try_from(setup_cmd: SetupCmd) -> Result<Self, Self::Error> {
        if let Some(bucket_identifier) = &setup_cmd.aws_s3_args.bucket_identifier {
            let identifier = ARN::parse(bucket_identifier).map(AWSResourceIdentifier::ARN).unwrap_or_else(|_| {
                let name = setup_cmd.aws_config_args.aws_prefix.map_or(bucket_identifier.clone(), |prefix| {
                    if !prefix.is_empty() {
                        StorageArgs::format_prefix_and_name(&prefix, bucket_identifier)
                    } else {
                        bucket_identifier.to_string()
                    }
                });
                AWSResourceIdentifier::Name(name)
            });

            Ok(Self { bucket_identifier: identifier })
        } else {
            Err(OrchestratorError::SetupCommandError("Missing bucket name".to_string()))
        }
    }
}

impl TryFrom<RunCmd> for StorageArgs {
    type Error = OrchestratorError;
    fn try_from(run_cmd: RunCmd) -> Result<Self, Self::Error> {
        if let Some(bucket_identifier) = &run_cmd.aws_s3_args.bucket_identifier {
            let identifier = ARN::parse(bucket_identifier).map(AWSResourceIdentifier::ARN).unwrap_or_else(|_| {
                let name = run_cmd.aws_config_args.aws_prefix.map_or(bucket_identifier.clone(), |prefix| {
                    if !prefix.is_empty() {
                        StorageArgs::format_prefix_and_name(&prefix, bucket_identifier)
                    } else {
                        bucket_identifier.to_string()
                    }
                });
                AWSResourceIdentifier::Name(name)
            });

            Ok(Self { bucket_identifier: identifier })
        } else {
            Err(OrchestratorError::RunCommandError("Missing bucket name".to_string()))
        }
    }
}

impl TryFrom<SetupCmd> for AlertArgs {
    type Error = OrchestratorError;
    fn try_from(setup_cmd: SetupCmd) -> Result<Self, Self::Error> {
        if let Some(topic_identifier) = &setup_cmd.aws_sns_args.topic_identifier {
            let identifier = ARN::parse(topic_identifier).map(AWSResourceIdentifier::ARN).unwrap_or_else(|_| {
                let name = setup_cmd.aws_config_args.aws_prefix.map_or(topic_identifier.clone(), |prefix| {
                    if !prefix.is_empty() {
                        AlertArgs::format_prefix_and_name(&prefix, topic_identifier)
                    } else {
                        topic_identifier.to_string()
                    }
                });
                AWSResourceIdentifier::Name(name)
            });

            Ok(Self { alert_identifier: identifier })
        } else {
            Err(OrchestratorError::SetupCommandError("Missing alert name".to_string()))
        }
    }
}

impl TryFrom<RunCmd> for AlertArgs {
    type Error = OrchestratorError;
    fn try_from(run_cmd: RunCmd) -> Result<Self, Self::Error> {
        if let Some(topic_identifier) = &run_cmd.aws_sns_args.topic_identifier {
            let identifier = ARN::parse(topic_identifier).map(AWSResourceIdentifier::ARN).unwrap_or_else(|_| {
                let name = run_cmd.aws_config_args.aws_prefix.map_or(topic_identifier.clone(), |prefix| {
                    if !prefix.is_empty() {
                        AlertArgs::format_prefix_and_name(&prefix, topic_identifier)
                    } else {
                        topic_identifier.to_string()
                    }
                });
                AWSResourceIdentifier::Name(name)
            });

            Ok(Self { alert_identifier: identifier })
        } else {
            Err(OrchestratorError::RunCommandError("Missing alert name".to_string()))
        }
    }
}

impl TryFrom<SetupCmd> for QueueArgs {
    type Error = OrchestratorError;
    fn try_from(setup_cmd: SetupCmd) -> Result<Self, Self::Error> {
        if let Some(queue_identifier) = &setup_cmd.aws_sqs_args.queue_identifier {
            let identifier = ARN::parse(queue_identifier).map(AWSResourceIdentifier::ARN).unwrap_or_else(|_| {
                let name = setup_cmd.aws_config_args.aws_prefix.map_or(queue_identifier.clone(), |prefix| {
                    if !prefix.is_empty() {
                        QueueArgs::format_prefix_and_name(&prefix, queue_identifier)
                    } else {
                        queue_identifier.to_string()
                    }
                });
                AWSResourceIdentifier::Name(name)
            });

            Ok(Self { queue_template_identifier: identifier })
        } else {
            Err(OrchestratorError::SetupCommandError("Missing queue template name".to_string()))
        }
    }
}

impl TryFrom<RunCmd> for QueueArgs {
    type Error = OrchestratorError;
    fn try_from(run_cmd: RunCmd) -> Result<Self, Self::Error> {
        if let Some(queue_identifier) = &run_cmd.aws_sqs_args.queue_identifier {
            let identifier = ARN::parse(queue_identifier).map(AWSResourceIdentifier::ARN).unwrap_or_else(|_| {
                let name = run_cmd.aws_config_args.aws_prefix.map_or(queue_identifier.clone(), |prefix| {
                    if !prefix.is_empty() {
                        QueueArgs::format_prefix_and_name(&prefix, queue_identifier)
                    } else {
                        queue_identifier.to_string()
                    }
                });
                AWSResourceIdentifier::Name(name)
            });

            Ok(Self { queue_template_identifier: identifier })
        } else {
            Err(OrchestratorError::SetupCommandError("Missing queue template name".to_string()))
        }
    }
}

impl TryFrom<SetupCmd> for CronArgs {
    type Error = OrchestratorError;
    fn try_from(setup_cmd: SetupCmd) -> Result<Self, Self::Error> {
        let target_queue_identifier = if let Some(queue_identifier) = &setup_cmd.aws_sqs_args.queue_identifier {
            ARN::parse(queue_identifier)
                .map(|arn| {
                    // creating queue with it's type name
                    let queue_name = InnerSQS::get_queue_name_from_type(&arn.resource, &QueueType::WorkerTrigger);
                    let updated_arn = ARN {
                        partition: arn.partition,
                        service: arn.service,
                        region: arn.region,
                        account_id: arn.account_id,
                        resource: queue_name,
                    };
                    AWSResourceIdentifier::ARN(updated_arn)
                })
                .unwrap_or_else(|_| {
                    let name =
                        setup_cmd.aws_config_args.aws_prefix.clone().map_or(queue_identifier.clone(), |prefix| {
                            if !prefix.is_empty() {
                                QueueArgs::format_prefix_and_name(&prefix, queue_identifier)
                            } else {
                                queue_identifier.to_string()
                            }
                        });
                    let updated_name = InnerSQS::get_queue_name_from_type(&name, &QueueType::WorkerTrigger);
                    AWSResourceIdentifier::Name(updated_name)
                })
        } else {
            return Err(OrchestratorError::SetupCommandError("Missing queue template name".to_string()));
        };

        // Create the RULE, ROLE, POLICY with format : {aws_prefix}-{mo-wt}-{rule/role/policy}
        // mo-wt stands for madara-orchestrator worker trigger.

        let prefix_str = match setup_cmd.aws_config_args.aws_prefix.as_deref() {
            Some(prefix) if !prefix.is_empty() => format!("{}-", prefix),
            _ => String::new(),
        };

        let trigger_rule_template_name = format!("{}mo-wt-rule", prefix_str);
        let trigger_role_name = format!("{}mo-wt-role", prefix_str);
        let trigger_policy_name = format!("{}mo-wt-policy", prefix_str);

        Ok(Self {
            target_queue_identifier,
            trigger_role_name,
            trigger_rule_template_name,
            trigger_policy_name,
            event_bridge_type: setup_cmd
                .aws_event_bridge_args
                .event_bridge_type
                .ok_or(OrchestratorError::SetupCommandError("Event Bridge type is required".to_string()))?,
            cron_time: setup_cmd
                .aws_event_bridge_args
                .interval_seconds
                .ok_or(OrchestratorError::SetupCommandError("Cron time is required".to_string()))?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case("arn:aws:sqs:us-east-1:000000000000:my-queue", "aws", "sqs", "us-east-1", "000000000000", "my-queue")]
    #[case(
        "arn:aws:sqs:us-west-2:000000000000:my-fifo-queue.fifo",
        "aws",
        "sqs",
        "us-west-2",
        "000000000000",
        "my-fifo-queue.fifo"
    )]
    #[case("arn:aws:sns:us-east-1:000000000000:my-topic", "aws", "sns", "us-east-1", "000000000000", "my-topic")]
    #[case(
        "arn:aws:sns:eu-west-1:111122223333:notification-topic-2024",
        "aws",
        "sns",
        "eu-west-1",
        "111122223333",
        "notification-topic-2024"
    )]
    #[case("arn:aws:iam::000000000000:user/john-doe", "aws", "iam", "", "000000000000", "user/john-doe")]
    #[case("arn:aws:iam::000000000000:role/MyRole", "aws", "iam", "", "000000000000", "role/MyRole")]
    #[case("arn:aws:iam::000000000000:policy/MyPolicy", "aws", "iam", "", "000000000000", "policy/MyPolicy")]
    #[case("arn:aws:iam::aws:policy/PowerUserAccess", "aws", "iam", "", "aws", "policy/PowerUserAccess")]
    #[case("arn:aws:s3:::my-bucket", "aws", "s3", "", "", "my-bucket")]
    #[case("arn:aws:s3:::my-bucket/path/to/object.txt", "aws", "s3", "", "", "my-bucket/path/to/object.txt")]
    fn test_arn_valid_works(
        #[case] arn_str: &str,
        #[case] expected_partition: &str,
        #[case] expected_service: &str,
        #[case] expected_region: &str,
        #[case] expected_account: &str,
        #[case] expected_resource: &str,
    ) {
        let arn = ARN::parse(arn_str).unwrap();
        assert_eq!(arn.partition, expected_partition);
        assert_eq!(arn.service, expected_service);
        assert_eq!(arn.region, expected_region);
        assert_eq!(arn.account_id, expected_account);
        assert_eq!(arn.resource, expected_resource);
    }

    #[rstest]
    #[case("")]
    #[case("   ")]
    #[case("arn2:aws:s3:::my-bucket")]
    #[case("arn:aws:s3")]
    #[case("arn:aws:s3:::my-bucket:extra:part")]
    #[case("arn::s3:::my-bucket")]
    #[case("arn:aws::::my-bucket")]
    #[case("arn:aws:sqs::000000000000:my-queue")] // SQS missing region
    #[case("arn:aws:sqs:us-east-1::my-queue")] // SQS missing account
    #[case("arn:aws:sqs:us-east-1:000000000000:")] // SQS missing resource
    #[case("arn:aws:sns::000000000000:my-topic")] // SNS missing region
    #[case("arn:aws:sns:us-east-1::my-topic")] // SNS missing account
    #[case("arn:aws:sns:us-east-1:000000000000:")] // SNS missing resource
    #[case("arn:aws:iam::000000000000:")] // IAM missing resource
    #[case("arn:aws:s3:::")] // S3 missing resource
    fn test_arn_invalid_fails(#[case] arn_str: &str) {
        assert!(ARN::parse(arn_str).is_err());
    }

    #[rstest]
    #[case("arn:aws:sqs:us-east-1:000000000000:my-queue")]
    #[case("arn:aws:s3:::my-bucket")]
    #[case("arn:aws:iam::000000000000:user/john-doe")]
    #[case("arn:aws:sns:us-east-1:000000000000:my-topic")]
    fn test_arn_display_roundtrip(#[case] arn_str: &str) {
        let arn = ARN::parse(arn_str).unwrap();
        assert_eq!(arn.to_string(), arn_str);
    }

    #[rstest]
    #[case("arn:aws:sqs:us-east-1:000000000000:a", "a")] // Minimum resource
    #[case("arn:aws:s3:::my-bucket/folder/file-name_with.dots", "my-bucket/folder/file-name_with.dots")] // Special chars
    fn test_arn_edge_cases(#[case] arn_str: &str, #[case] expected_resource: &str) {
        let arn = ARN::parse(arn_str).unwrap();
        assert_eq!(arn.resource, expected_resource);
    }
}
