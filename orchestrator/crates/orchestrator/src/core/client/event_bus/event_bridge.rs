use crate::cli::cron::event_bridge::EventBridgeType;
use crate::core::client::event_bus::EventBusClient;
use crate::types::jobs::WorkerTriggerType;
use crate::types::params::CronArgs;
use anyhow::Error;
use aws_config::SdkConfig;
use aws_sdk_eventbridge::types::{InputTransformer, RuleState, Target as EventBridgeTarget};
use aws_sdk_scheduler::types::{FlexibleTimeWindow, FlexibleTimeWindowMode, Target};
use aws_sdk_sqs::types::QueueAttributeName;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct TriggerArns {
    queue_arn: String,
    role_arn: String,
}

/// EventBridge Client implementation
pub struct EventBridgeClient {
    pub(crate) eb_client: Arc<aws_sdk_eventbridge::Client>,
    pub(crate) scheduler_client: Arc<aws_sdk_scheduler::Client>,
    queue_client: Arc<aws_sdk_sqs::Client>,
    iam_client: Arc<aws_sdk_iam::Client>,
    pub event_bridge_type: Option<EventBridgeType>,
    pub target_queue_name: Option<String>,
    pub cron_time: Option<Duration>,
    pub trigger_rule_name: Option<String>,
    pub trigger_role_name: Option<String>,
    pub trigger_policy_name: Option<String>,
}

impl EventBridgeClient {
    /// new - Create a new EventBridge client, with both client and option for the client;
    /// we've needed to pass the aws_config and args to the constructor.
    /// # Arguments
    /// * `aws_config` - The AWS configuration.
    /// * `args` - The cron arguments.
    /// # Returns
    /// * `Self` - The EventBridge client.
    pub fn new(aws_config: &SdkConfig, args: Option<&CronArgs>) -> Self {
        Self {
            eb_client: Arc::new(aws_sdk_eventbridge::Client::new(aws_config)),
            scheduler_client: Arc::new(aws_sdk_scheduler::Client::new(aws_config)),
            queue_client: Arc::new(aws_sdk_sqs::Client::new(aws_config)),
            iam_client: Arc::new(aws_sdk_iam::Client::new(aws_config)),
            event_bridge_type: args.map(|args| args.event_bridge_type.clone()),
            target_queue_name: args.map(|args| args.target_queue_name.clone()),
            cron_time: args
                .map(|args| args.cron_time.clone())
                .and_then(|cron_time| cron_time.parse::<u64>().ok())
                .map(Duration::from_secs),
            trigger_rule_name: args.map(|args| args.trigger_rule_name.clone()),
            trigger_role_name: args.map(|args| args.trigger_role_name.clone()),
            trigger_policy_name: args.map(|args| args.trigger_policy_name.clone()),
        }
    }

    /// get_queue_arn - Get the ARN of a queue
    ///
    /// # Arguments
    ///
    /// * `queue_name` - The name of the queue
    ///
    /// # Returns
    /// * `String` - The ARN of the queue
    async fn get_queue_arn(&self, queue_name: &str) -> Result<String, Error> {
        let queue_url = self.queue_client.get_queue_url().queue_name(queue_name).send().await?;
        let queue_attributes = self
            .queue_client
            .get_queue_attributes()
            .queue_url(queue_url.queue_url.unwrap())
            .attribute_names(QueueAttributeName::QueueArn)
            .send()
            .await?;
        queue_attributes
            .attributes()
            .and_then(|attrs| attrs.get(&QueueAttributeName::QueueArn))
            .map(String::from)
            .ok_or_else(|| Error::msg("Queue ARN not found"))
    }

    async fn create_iam_role(&self, role_name: &str) -> Result<String, Error> {
        let assume_role_policy = r#"{
            "Version": "2012-10-17",
            "Statement": [{
                "Effect": "Allow",
                "Principal": {
                    "Service": ["scheduler.amazonaws.com", "events.amazonaws.com"]
                },
                "Action": "sts:AssumeRole"
            }]
        }"#;
        let create_role_resp = self
            .iam_client
            .create_role()
            .role_name(role_name)
            .assume_role_policy_document(assume_role_policy)
            .send()
            .await?;
        let role = create_role_resp.role().ok_or_else(|| Error::msg("Failed to create IAM role"))?;
        Ok(role.arn().to_string())
    }

    async fn create_and_attach_sqs_policy(
        &self,
        policy_name: &str,
        role_name: &str,
        queue_arn: &str,
    ) -> Result<(), Error> {
        let policy_document = format!(
            r#"{{
            "Version": "2012-10-17",
            "Statement": [{{
                "Effect": "Allow",
                "Action": ["sqs:SendMessage"],
                "Resource": "{}"
            }}]
        }}"#,
            queue_arn
        );
        let create_policy_resp =
            self.iam_client.create_policy().policy_name(policy_name).policy_document(&policy_document).send().await?;
        let policy = create_policy_resp.policy().ok_or_else(|| Error::msg("Failed to create policy"))?;

        let policy_arn = policy.arn().ok_or_else(|| Error::msg("Failed to get policy ARN"))?;

        self.iam_client.attach_role_policy().role_name(role_name).policy_arn(policy_arn).send().await?;

        Ok(())
    }

    pub async fn create_cron(
        &self,
        target_queue_name: String,
        trigger_role_name: String,
        trigger_policy_name: String,
    ) -> Result<TriggerArns, Error> {
        let queue_arn = self.get_queue_arn(&target_queue_name).await?;

        let role_name = format!("{}-{}", trigger_role_name, uuid::Uuid::new_v4());
        let role_arn = self.create_iam_role(&role_name).await?;

        let policy_name = format!("{}-{}", trigger_policy_name, uuid::Uuid::new_v4());
        self.create_and_attach_sqs_policy(&policy_name, &role_name, &queue_arn).await?;

        Ok(TriggerArns { queue_arn, role_arn })
    }

    /// duration_to_rate_string - Converts a Duration to a rate string for AWS EventBridge
    ///
    /// # Arguments
    ///
    /// * `duration` - The duration to convert
    ///
    /// # Returns
    ///
    /// * `String` - The rate string in the format "rate(X unit)" where X is the number of units
    ///
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
    /// TODO: we might need to move this code to Setup since there is not use case for this function in client
    pub(crate) async fn add_cron_target_queue(
        &self,
        trigger_type: &WorkerTriggerType,
        trigger_arns: &TriggerArns,
        trigger_rule_name: String,
        event_bridge_type: EventBridgeType,
        cron_time: Duration,
    ) -> color_eyre::Result<()> {
        let message = trigger_type.clone().to_string();
        let trigger_name = format!("{}-{}", trigger_rule_name.clone(), trigger_type);

        match event_bridge_type.clone() {
            EventBridgeType::Rule => {
                let input_transformer =
                    InputTransformer::builder().input_paths_map("time", "$.time").input_template(message).build()?;

                self.eb_client
                    .put_rule()
                    .name(trigger_name.clone())
                    .schedule_expression("rate(1 minute)")
                    .state(RuleState::Enabled)
                    .send()
                    .await?;

                self.eb_client
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
            EventBridgeType::Schedule => {
                // Set flexible time window (you can adjust this as needed)
                let flexible_time_window = FlexibleTimeWindow::builder().mode(FlexibleTimeWindowMode::Off).build()?;

                let message = trigger_type.clone().to_string();

                // Create target for SQS queue
                let target = Target::builder()
                    .arn(trigger_arns.queue_arn.clone())
                    .role_arn(trigger_arns.role_arn.clone())
                    .input(message)
                    .build()?;

                // Create the schedule
                self.scheduler_client
                    .create_schedule()
                    .name(trigger_name)
                    .schedule_expression_timezone("UTC")
                    .flexible_time_window(flexible_time_window)
                    .schedule_expression(Self::duration_to_rate_string(cron_time))
                    .target(target)
                    .send()
                    .await?;
            }
        };

        Ok(())
    }
}

#[async_trait::async_trait]
impl EventBusClient for EventBridgeClient {}
