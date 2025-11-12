use rstest::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::client::queue::sqs::{InnerSQS, SQS};
    use crate::core::client::queue::QueueClient;
    use crate::types::params::{AWSResourceIdentifier, QueueArgs};
    use crate::types::queue::QueueType;
    use aws_config::{BehaviorVersion, Region};
    use orchestrator_utils::env_utils::get_env_var_or_panic;

    /// Fixture to create localstack AWS config
    #[fixture]
    async fn localstack_config() -> aws_config::SdkConfig {
        dotenvy::from_filename_override("../.env.test").expect("Failed to load the .env.test file");
        let aws_endpoint_url = get_env_var_or_panic("AWS_ENDPOINT_URL");
        aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .endpoint_url(aws_endpoint_url)
            .load()
            .await
    }

    /// Fixture to create a test queue with InnerSQS
    /// Returns tuple of (InnerSQS, queue_template, queue_url)
    #[fixture]
    async fn test_queue(#[future] localstack_config: aws_config::SdkConfig) -> (InnerSQS, String, String) {
        let config = localstack_config.await;
        // Create queue name with template format: "test-{uuid}-{}_queue"
        // This matches the format expected by get_queue_name_from_type
        let uuid = uuid::Uuid::new_v4();
        let uuid_str = uuid.to_string();
        let queue_template = format!("test-{}-{{}}_queue", &uuid_str[..8]);
        let queue_name = InnerSQS::get_queue_name_from_type(&queue_template, &QueueType::SnosJobProcessing);

        let inner_sqs = InnerSQS::new(&config);
        inner_sqs.client().create_queue().queue_name(&queue_name).send().await.expect("Failed to create queue");

        // Wait a bit for queue to be fully available
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let queue_url = inner_sqs.get_queue_url_from_client(&queue_name).await.expect("Failed to get queue URL");

        // Create QueueArgs with the template (not the specific queue name)
        (inner_sqs, queue_template, queue_url)
    }

    /// Test that send_message adds the OrchestratorVersion message attribute using Omni Queue
    #[rstest]
    #[tokio::test]
    async fn test_send_message_with_version_attribute(
        #[future] localstack_config: aws_config::SdkConfig,
        #[future] test_queue: (InnerSQS, String, String),
    ) {
        let config = localstack_config.await;
        let (inner_sqs, queue_template, _queue_url) = test_queue.await;

        // Create SQS client using our QueueClient abstraction
        let queue_args = QueueArgs { queue_template_identifier: AWSResourceIdentifier::Name(queue_template.clone()) };
        let sqs = SQS::new(&config, &queue_args);

        // Get the actual queue name and URL for this specific queue type
        let actual_queue_name = InnerSQS::get_queue_name_from_type(&queue_template, &QueueType::SnosJobProcessing);
        let _actual_queue_url =
            inner_sqs.get_queue_url_from_client(&actual_queue_name).await.expect("Failed to get queue URL");

        // Send message using our QueueClient (which adds version attribute)
        let test_payload = "test message content";
        sqs.send_message(QueueType::SnosJobProcessing, test_payload.to_string(), None)
            .await
            .expect("Failed to send message");

        // Receive message using Omni Queue consumer
        let delivery =
            sqs.consume_message_from_queue(QueueType::SnosJobProcessing).await.expect("Failed to consume message");

        // Verify message payload
        let payload_bytes = delivery.borrow_payload().expect("Failed to get payload");
        let payload = String::from_utf8(payload_bytes.to_vec()).expect("Failed to parse payload as UTF-8");
        assert_eq!(payload, test_payload);

        // Acknowledge the message
        delivery.ack().await.expect("Failed to ack message");

        // Cleanup - delete the queue we created
        let actual_queue_name = InnerSQS::get_queue_name_from_type(&queue_template, &QueueType::SnosJobProcessing);
        let actual_queue_url =
            inner_sqs.get_queue_url_from_client(&actual_queue_name).await.expect("Failed to get queue URL");
        inner_sqs.client().delete_queue().queue_url(&actual_queue_url).send().await.ok();
    }

    /// Integration test for version-based message filtering
    /// Tests that consumers only receive messages matching their orchestrator version
    /// and incompatible messages are NOT consumed (remain in queue or go to DLQ)
    #[rstest]
    #[tokio::test]
    async fn test_version_filtering_consumes_only_compatible_messages(
        #[future] localstack_config: aws_config::SdkConfig,
        #[future] test_queue: (InnerSQS, String, String),
    ) {
        use crate::core::cloud::CloudProvider;
        use crate::core::config::Config;
        use crate::types::constant::ORCHESTRATOR_VERSION_ATTRIBUTE;
        use aws_sdk_sqs::types::MessageAttributeValue;
        use std::sync::Arc;
        use tokio::time::{sleep, Duration};

        let config = localstack_config.await;
        let (inner_sqs, queue_template, _queue_url) = test_queue.await;

        // Create CloudProvider and use Config to build queue client (proper way for tests)
        let provider_config = Arc::new(CloudProvider::AWS(Box::new(config.clone())));
        let queue_args = QueueArgs { queue_template_identifier: AWSResourceIdentifier::Name(queue_template.clone()) };
        let sqs =
            Config::build_queue_client(&queue_args, provider_config).await.expect("Failed to create queue client");

        // Get the actual queue name and URL for this specific queue type
        let actual_queue_name = InnerSQS::get_queue_name_from_type(&queue_template, &QueueType::SnosJobProcessing);
        let actual_queue_url =
            inner_sqs.get_queue_url_from_client(&actual_queue_name).await.expect("Failed to get queue URL");

        let incompatible_version = "orchestrator-0.0.1";

        // Send 5 messages with incompatible version (should NOT be consumed)
        for i in 0..5 {
            let payload = format!("incompatible-message-{}", i);
            let version_attr = MessageAttributeValue::builder()
                .data_type("String")
                .string_value(incompatible_version)
                .build()
                .expect("Failed to build version attribute");

            inner_sqs
                .client()
                .send_message()
                .queue_url(&actual_queue_url)
                .message_body(&payload)
                .message_attributes(ORCHESTRATOR_VERSION_ATTRIBUTE, version_attr)
                .send()
                .await
                .expect("Failed to send incompatible message");
        }

        // Send 10 messages with current version (should be consumed)
        for i in 0..10 {
            let payload = format!("compatible-message-{}", i);
            sqs.send_message(QueueType::SnosJobProcessing, payload, None)
                .await
                .expect("Failed to send compatible message");
        }

        // Give a moment for messages to be available
        sleep(Duration::from_millis(100)).await;

        // Track messages received
        let mut compatible_received = 0;
        let mut incompatible_received = 0;

        // Consume messages with timeout
        let start = tokio::time::Instant::now();
        let timeout = Duration::from_secs(10);

        while start.elapsed() < timeout && compatible_received < 10 {
            match sqs.consume_message_from_queue(QueueType::SnosJobProcessing).await {
                Ok(delivery) => {
                    // Verify we got a message
                    let payload_bytes = delivery.borrow_payload().expect("Failed to get payload");
                    let payload = String::from_utf8(payload_bytes.to_vec()).expect("Failed to parse payload as UTF-8");

                    if payload.contains("compatible-message") {
                        println!("Received compatible message: {}", payload);
                        compatible_received += 1;
                    } else if payload.contains("incompatible-message") {
                        println!("Received incompatible message: {}", payload);
                        incompatible_received += 1;
                    }

                    // Acknowledge the message
                    delivery.ack().await.expect("Failed to ack message");
                }
                Err(e) => {
                    // Check if it's a NoData error (version mismatch or no messages)
                    if e.to_string().contains("NoData") {
                        sleep(Duration::from_millis(100)).await;
                    } else {
                        println!("Error: {}", e);
                        sleep(Duration::from_millis(100)).await;
                    }
                }
            }
        }

        println!("Compatible messages received: {}", compatible_received);
        println!("Incompatible messages received: {}", incompatible_received);

        assert_eq!(compatible_received, 10, "Should have received all 10 compatible messages");
        assert_eq!(incompatible_received, 0, "Should NOT have received any incompatible messages");

        // Validate that exactly 5 incompatible messages remain in the queue
        let queue_attributes = inner_sqs
            .client()
            .get_queue_attributes()
            .queue_url(&actual_queue_url)
            .attribute_names(aws_sdk_sqs::types::QueueAttributeName::ApproximateNumberOfMessages)
            .send()
            .await
            .expect("Failed to get queue attributes");

        let approximate_message_count = queue_attributes
            .attributes()
            .and_then(|attrs| attrs.get(&aws_sdk_sqs::types::QueueAttributeName::ApproximateNumberOfMessages))
            .and_then(|count| count.parse::<i32>().ok())
            .expect("Failed to get approximate message count");

        println!("Have exactly {} incompatible messages remaining in queue", approximate_message_count);

        assert_eq!(approximate_message_count, 5, "Should have exactly 5 incompatible messages remaining in queue");

        // Cleanup
        inner_sqs.client().delete_queue().queue_url(&actual_queue_url).send().await.ok();
    }
}
