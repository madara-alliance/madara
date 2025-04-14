use crate::cli::cron::event_bridge::EventBridgeType;
use crate::core::client::cron::CronClient;
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
    eb_client: Arc<aws_sdk_eventbridge::Client>,
    scheduler_client: Arc<aws_sdk_scheduler::Client>,
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
    pub fn constructor(
        eb_client: Arc<aws_sdk_eventbridge::Client>,
        scheduler_client: Arc<aws_sdk_scheduler::Client>,
        queue_client: Arc<aws_sdk_sqs::Client>,
        iam_client: Arc<aws_sdk_iam::Client>,
    ) -> Self {
        EventBridgeClient {
            eb_client,
            scheduler_client,
            queue_client,
            iam_client,
            event_bridge_type: None,
            target_queue_name: None,
            cron_time: None,
            trigger_rule_name: None,
            trigger_role_name: None,
            trigger_policy_name: None,
        }
    }
    pub fn create(args: &CronArgs, aws_config: &SdkConfig) -> Self {
        Self {
            eb_client: Arc::new(aws_sdk_eventbridge::Client::new(aws_config)),
            scheduler_client: Arc::new(aws_sdk_scheduler::Client::new(aws_config)),
            queue_client: Arc::new(aws_sdk_sqs::Client::new(aws_config)),
            iam_client: Arc::new(aws_sdk_iam::Client::new(aws_config)),
            event_bridge_type: Some(args.event_bridge_type.clone()),
            target_queue_name: Some(args.target_queue_name.clone()),
            cron_time: Some(Duration::from_secs(
                args.cron_time.clone().parse::<u64>().expect("Failed to parse cron time"),
            )),
            trigger_rule_name: Some(args.trigger_rule_name.clone()),
            trigger_role_name: Some(args.trigger_role_name.clone()),
            trigger_policy_name: Some(args.trigger_policy_name.clone()),
        }
    }

    pub async fn create_cron(
        &self,
        target_queue_name: String,
        trigger_role_name: String,
        trigger_policy_name: String,
    ) -> Result<TriggerArns, Error> {
        // Get Queue Info
        let queue_url = self.queue_client.get_queue_url().queue_name(target_queue_name.clone()).send().await?;

        let queue_attributes = self
            .queue_client
            .get_queue_attributes()
            .queue_url(queue_url.queue_url.unwrap())
            .attribute_names(QueueAttributeName::QueueArn)
            .send()
            .await?;
        let queue_arn = queue_attributes.attributes().unwrap().get(&QueueAttributeName::QueueArn).unwrap();

        // Create IAM role for EventBridge
        let role_name = format!("{}-{}", trigger_role_name.clone(), uuid::Uuid::new_v4());
        // TODO: might need to change this accordingly to support rule, skipping for now
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

        let policy_name = format!("{}-{}", trigger_policy_name.clone(), uuid::Uuid::new_v4());

        // Create and attach the policy
        let policy_resp =
            self.iam_client.create_policy().policy_name(&policy_name).policy_document(&policy_document).send().await?;

        let policy_arn = policy_resp.policy().unwrap().arn().unwrap().to_string();

        // Attach the policy to the role
        self.iam_client.attach_role_policy().role_name(&role_name).policy_arn(&policy_arn).send().await?;

        // sleep(Duration::from_secs(60)).await;

        Ok(TriggerArns { queue_arn: queue_arn.to_string(), role_arn: role_arn.to_string() })
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
}

#[async_trait::async_trait]
impl CronClient for EventBridgeClient {
    async fn add_cron_target_queue(
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
