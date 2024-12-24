use std::time::Duration;

use async_trait::async_trait;
use aws_config::SdkConfig;
use aws_sdk_eventbridge::types::{InputTransformer, RuleState, Target as EventBridgeTarget};
use aws_sdk_scheduler::types::{FlexibleTimeWindow, FlexibleTimeWindowMode, Target};
use aws_sdk_sqs::types::QueueAttributeName;
use aws_sdk_sqs::Client as SqsClient;
use color_eyre::eyre::Ok;

use super::{get_worker_trigger_message, TriggerArns};
use crate::cron::Cron;
use crate::queue::job_queue::WorkerTriggerType;

#[derive(Clone, Debug, clap::ValueEnum)]
pub enum EventBridgeType {
    Rule,
    Schedule,
}

#[derive(Clone, Debug)]
enum EventBridgeClient {
    Rule(aws_sdk_eventbridge::Client),
    Schedule(aws_sdk_scheduler::Client),
}

#[derive(Clone, Debug)]
pub struct AWSEventBridgeValidatedArgs {
    pub cron_type: EventBridgeType,
    pub target_queue_name: String,
    pub cron_time: Duration,
    pub trigger_rule_name: String,
    pub trigger_role_name: String,
    pub trigger_policy_name: String,
}

pub struct AWSEventBridge {
    target_queue_name: String,
    cron_time: Duration,
    trigger_rule_name: String,
    client: EventBridgeClient,
    queue_client: SqsClient,
    iam_client: aws_sdk_iam::Client,
    trigger_role_name: String,
    trigger_policy_name: String,
}

impl AWSEventBridge {
    pub fn new_with_args(params: &AWSEventBridgeValidatedArgs, aws_config: &SdkConfig) -> Self {
        let client = match params.cron_type {
            EventBridgeType::Rule => EventBridgeClient::Rule(aws_sdk_eventbridge::Client::new(aws_config)),
            EventBridgeType::Schedule => EventBridgeClient::Schedule(aws_sdk_scheduler::Client::new(aws_config)),
        };

        Self {
            target_queue_name: params.target_queue_name.clone(),
            cron_time: params.cron_time,
            trigger_rule_name: params.trigger_rule_name.clone(),
            client,
            queue_client: aws_sdk_sqs::Client::new(aws_config),
            iam_client: aws_sdk_iam::Client::new(aws_config),
            trigger_role_name: params.trigger_role_name.clone(),
            trigger_policy_name: params.trigger_policy_name.clone(),
        }
    }
}

#[async_trait]
#[allow(unreachable_patterns)]
impl Cron for AWSEventBridge {
    async fn create_cron(&self) -> color_eyre::Result<TriggerArns> {
        // Get Queue Info
        let queue_url = self.queue_client.get_queue_url().queue_name(&self.target_queue_name).send().await?;

        let queue_attributes = self
            .queue_client
            .get_queue_attributes()
            .queue_url(queue_url.queue_url.unwrap())
            .attribute_names(QueueAttributeName::QueueArn)
            .send()
            .await?;
        let queue_arn = queue_attributes.attributes().unwrap().get(&QueueAttributeName::QueueArn).unwrap();

        // Create IAM role for EventBridge
        let role_name = format!("{}-{}", self.trigger_role_name, uuid::Uuid::new_v4());
        // TODO: might need to change this accordingly to support rule, skipping for now
        let assume_role_policy = r#"{
            "Version": "2012-10-17",
            "Statement": [{
                "Effect": "Allow",
                "Principal": {
                    "Service": "scheduler.amazonaws.com"
                },
                "Action": "sts:AssumeRole"
            }]
        }"#;

        let create_role_resp = self
            .iam_client
            .create_role()
            .role_name(&role_name)
            .assume_role_policy_document(assume_role_policy)
            .send()
            .await?;

        let role_arn = create_role_resp.role().unwrap().arn();

        // Create policy document for SQS access
        let policy_document = format!(
            r#"{{
            "Version": "2012-10-17",
            "Statement": [{{
                "Effect": "Allow",
                "Action": [
                    "sqs:SendMessage"
                ],
                "Resource": "{}"
            }}]
        }}"#,
            queue_arn
        );

        let policy_name = format!("{}-{}", self.trigger_policy_name, uuid::Uuid::new_v4());

        // Create and attach the policy
        let policy_resp =
            self.iam_client.create_policy().policy_name(&policy_name).policy_document(&policy_document).send().await?;

        let policy_arn = policy_resp.policy().unwrap().arn().unwrap().to_string();

        // Attach the policy to the role
        self.iam_client.attach_role_policy().role_name(&role_name).policy_arn(&policy_arn).send().await?;

        // sleep(Duration::from_secs(60)).await;

        Ok(TriggerArns { queue_arn: queue_arn.to_string(), role_arn: role_arn.to_string() })
    }

    async fn add_cron_target_queue(
        &self,
        trigger_type: &WorkerTriggerType,
        trigger_arns: &TriggerArns,
    ) -> color_eyre::Result<()> {
        let message = get_worker_trigger_message(trigger_type.clone())?;
        let trigger_name = format!("{}-{}", self.trigger_rule_name, trigger_type);
        println!("trigger_nametrigger_nametrigger_name {}", trigger_name);

        match self.client.clone() {
            EventBridgeClient::Rule(client) => {
                let input_transformer =
                    InputTransformer::builder().input_paths_map("time", "$.time").input_template(message).build()?;

                client
                    .put_rule()
                    .name(trigger_name.clone())
                    .schedule_expression("rate(1 minute)")
                    .state(RuleState::Enabled)
                    .send()
                    .await?;

                client
                    .put_targets()
                    .rule(trigger_name.clone())
                    .targets(
                        EventBridgeTarget::builder()
                            .id(uuid::Uuid::new_v4().to_string())
                            .arn(trigger_arns.queue_arn.clone())
                            .input_transformer(input_transformer.clone())
                            .build()?,
                    )
                    .send()
                    .await?;
            }
            EventBridgeClient::Schedule(client) => {
                // Set flexible time window (you can adjust this as needed)
                let flexible_time_window = FlexibleTimeWindow::builder().mode(FlexibleTimeWindowMode::Off).build()?;

                let message = get_worker_trigger_message(trigger_type.clone())?;

                // Create target for SQS queue
                let target = Target::builder()
                    .arn(trigger_arns.queue_arn.clone())
                    .role_arn(trigger_arns.role_arn.clone())
                    .input(message)
                    .build()?;

                // Create the schedule
                client
                    .create_schedule()
                    .name(trigger_name)
                    .schedule_expression_timezone("UTC")
                    .flexible_time_window(flexible_time_window)
                    .schedule_expression(duration_to_rate_string(self.cron_time))
                    .target(target)
                    .send()
                    .await?;
            }
        };

        Ok(())
    }
}

fn duration_to_rate_string(duration: Duration) -> String {
    let total_secs = duration.as_secs();
    let total_mins = duration.as_secs() / 60;
    let total_hours = duration.as_secs() / 3600;
    let total_days = duration.as_secs() / 86400;

    if total_days > 0 {
        format!("rate({} day{})", total_days, if total_days == 1 { "" } else { "s" })
    } else if total_hours > 0 {
        format!("rate({} hour{})", total_hours, if total_hours == 1 { "" } else { "s" })
    } else if total_mins > 0 {
        format!("rate({} minute{})", total_mins, if total_mins == 1 { "" } else { "s" })
    } else {
        format!("rate({} second{})", total_secs, if total_secs == 1 { "" } else { "s" })
    }
}

#[cfg(test)]
mod event_bridge_utils_test {
    use rstest::rstest;

    use super::*;

    #[rstest]
    fn test_duration_to_rate_string() {
        assert_eq!(duration_to_rate_string(Duration::from_secs(60)), "rate(1 minute)");
        assert_eq!(duration_to_rate_string(Duration::from_secs(120)), "rate(2 minutes)");
        assert_eq!(duration_to_rate_string(Duration::from_secs(30)), "rate(30 seconds)");
        assert_eq!(duration_to_rate_string(Duration::from_secs(3600)), "rate(1 hour)");
        assert_eq!(duration_to_rate_string(Duration::from_secs(86400)), "rate(1 day)");
    }
}
