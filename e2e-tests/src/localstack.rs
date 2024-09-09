use aws_config::Region;
use aws_sdk_eventbridge::types::{InputTransformer, RuleState, Target};
use aws_sdk_sqs::types::QueueAttributeName;
use aws_sdk_sqs::types::QueueAttributeName::VisibilityTimeout;
use orchestrator::config::ProviderConfig;
use orchestrator::data_storage::aws_s3::AWSS3;
use orchestrator::data_storage::DataStorage;
use orchestrator::queue::job_queue::{
    WorkerTriggerMessage, WorkerTriggerType, JOB_HANDLE_FAILURE_QUEUE, JOB_PROCESSING_QUEUE, JOB_VERIFICATION_QUEUE,
    WORKER_TRIGGER_QUEUE,
};
use std::collections::HashMap;
use std::sync::Arc;
use utils::env_utils::get_env_var_or_panic;
use utils::settings::env::EnvSettingsProvider;

/// LocalStack struct
pub struct LocalStack {
    sqs_client: aws_sdk_sqs::Client,
    pub s3_client: Box<dyn DataStorage + Send + Sync>,
    event_bridge_client: aws_sdk_eventbridge::Client,
}

impl LocalStack {
    pub async fn new() -> Self {
        let region_provider = Region::new(get_env_var_or_panic("AWS_REGION"));
        let config = aws_config::from_env().region(region_provider).load().await;

        Self {
            sqs_client: aws_sdk_sqs::Client::new(&config),
            s3_client: Box::new(
                AWSS3::new_with_settings(&EnvSettingsProvider {}, ProviderConfig::AWS(Arc::from(config.clone()))).await,
            ),
            event_bridge_client: aws_sdk_eventbridge::Client::new(&config),
        }
    }

    #[allow(clippy::borrowed_box)]
    pub fn s3_client(&self) -> &Box<dyn DataStorage + Send + Sync> {
        &self.s3_client
    }

    /// To set up SQS on localstack instance
    pub async fn setup_sqs(&self) -> color_eyre::Result<()> {
        let list_queues_output = self.sqs_client.list_queues().send().await?;
        let queue_urls = list_queues_output.queue_urls();
        log::debug!("Found {} queues", queue_urls.len());
        for queue_url in queue_urls {
            match self.sqs_client.delete_queue().queue_url(queue_url).send().await {
                Ok(_) => log::debug!("Successfully deleted queue: {}", queue_url),
                Err(e) => eprintln!("Error deleting queue {}: {:?}", queue_url, e),
            }
        }

        // Creating SQS queues
        let mut queue_attributes = HashMap::new();
        queue_attributes.insert(VisibilityTimeout, "1".into());
        self.sqs_client
            .create_queue()
            .queue_name(JOB_PROCESSING_QUEUE)
            .set_attributes(Some(queue_attributes.clone()))
            .send()
            .await?;
        self.sqs_client
            .create_queue()
            .queue_name(JOB_VERIFICATION_QUEUE)
            .set_attributes(Some(queue_attributes.clone()))
            .send()
            .await?;
        self.sqs_client
            .create_queue()
            .queue_name(JOB_HANDLE_FAILURE_QUEUE)
            .set_attributes(Some(queue_attributes.clone()))
            .send()
            .await?;
        self.sqs_client
            .create_queue()
            .queue_name(WORKER_TRIGGER_QUEUE)
            .set_attributes(Some(queue_attributes.clone()))
            .send()
            .await?;
        println!("🌊 SQS queues creation completed.");

        Ok(())
    }

    /// Event Bridge setup
    pub async fn setup_event_bridge(&self, worker_trigger_type: WorkerTriggerType) -> color_eyre::Result<()> {
        let rule_name = "worker_trigger_scheduled";

        self.event_bridge_client
            .put_rule()
            .name(rule_name)
            .schedule_expression("rate(1 minute)")
            .state(RuleState::Enabled)
            .send()
            .await?;
        let queue_url = self.sqs_client.get_queue_url().queue_name(WORKER_TRIGGER_QUEUE).send().await?;

        let queue_attributes = self
            .sqs_client
            .get_queue_attributes()
            .queue_url(queue_url.queue_url.unwrap())
            .attribute_names(QueueAttributeName::QueueArn)
            .send()
            .await?;
        let queue_arn = queue_attributes.attributes().unwrap().get(&QueueAttributeName::QueueArn).unwrap();

        // Create a sample WorkerTriggerMessage
        let message = WorkerTriggerMessage { worker: worker_trigger_type.clone() };
        let event_detail = serde_json::to_string(&message)?;

        // Create the EventBridge target with the input transformer
        let input_transformer = InputTransformer::builder()
            .input_paths_map("$.time", "time")
            .input_template(event_detail.to_string())
            .build()?;

        self.event_bridge_client
            .put_targets()
            .rule(rule_name)
            .targets(
                Target::builder()
                    .id(format!("worker-trigger-target-{:?}", worker_trigger_type))
                    .arn(queue_arn)
                    .input_transformer(input_transformer)
                    .build()?,
            )
            .send()
            .await?;

        println!("🌉 Event bridge setup completed. Trigger Type : {:?}", worker_trigger_type);

        Ok(())
    }

    /// Generic function to send message to any of the queues
    pub async fn send_message_to_queue(&self, queue_url: &str, message_body: &str) -> color_eyre::Result<()> {
        self.sqs_client.send_message().queue_url(queue_url).message_body(message_body).send().await?;
        Ok(())
    }

    pub async fn delete_event_bridge_rule(&self, rule_name: &str) -> color_eyre::Result<()> {
        let list_targets_output = self.event_bridge_client.list_targets_by_rule().rule(rule_name).send().await;

        match list_targets_output {
            Ok(output) => {
                let targets = output.targets();
                if !targets.is_empty() {
                    let target_ids: Vec<String> = targets.iter().map(|t| t.id().to_string()).collect();

                    self.event_bridge_client.remove_targets().rule(rule_name).set_ids(Some(target_ids)).send().await?;

                    println!("🧹 Removed targets from rule: {}", rule_name);
                }

                // Step 2: Delete the rule
                self.event_bridge_client.delete_rule().name(rule_name).send().await?;

                println!("🧹 Deleted EventBridge rule: {}", rule_name);
                println!("🧹 Rule deleted successfully.");

                Ok(())
            }
            Err(_) => Ok(()),
        }
    }
}
