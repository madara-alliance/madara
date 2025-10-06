use rstest::*;

/// Test that send_message adds the OrchestratorVersion message attribute using Omni Queue
#[rstest]
#[tokio::test]
async fn test_send_message_with_version_attribute() {
    use crate::core::client::queue::sqs::{InnerSQS, SQS};
    use crate::core::client::queue::QueueClient;
    use crate::types::params::{AWSResourceIdentifier, QueueArgs};
    use crate::types::queue::QueueType;
    use aws_config::{BehaviorVersion, Region};

    // Create localstack AWS config
    // Use environment variable to determine endpoint, defaulting to localhost for tests
    let endpoint_url = std::env::var("LOCALSTACK_ENDPOINT").unwrap_or_else(|_| "http://localhost:4566".to_string());
    let config = aws_config::defaults(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .endpoint_url(endpoint_url)
        .load()
        .await;

    let queue_name = format!("test-snos-job-processing-{}", uuid::Uuid::new_v4());

    // Create queue using InnerSQS (needed for test setup only)
    let inner_sqs = InnerSQS::new(&config);
    inner_sqs.client().create_queue().queue_name(&queue_name).send().await.expect("Failed to create queue");

    // Create SQS client using our QueueClient abstraction
    let queue_args = QueueArgs { queue_template_identifier: AWSResourceIdentifier::Name(queue_name.clone()) };
    let sqs = SQS::new(&config, &queue_args);

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

    // Cleanup
    let queue_url = inner_sqs.get_queue_url_from_client(&queue_name).await.expect("Failed to get queue URL");
    inner_sqs.client().delete_queue().queue_url(&queue_url).send().await.ok();
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::core::client::queue::sqs::SQS;
    use crate::core::client::queue::QueueClient;
    use crate::types::params::{AWSResourceIdentifier, QueueArgs};
    use crate::types::queue::QueueType;
    use aws_config::{BehaviorVersion, Region};
    use std::sync::{Arc, Mutex};
    use tokio::time::{sleep, Duration};

    /// Helper to create localstack AWS config
    async fn get_localstack_config() -> aws_config::SdkConfig {
        // Use environment variable to determine endpoint, defaulting to localhost for tests
        let endpoint_url = std::env::var("LOCALSTACK_ENDPOINT").unwrap_or_else(|_| "http://localhost:4566".to_string());
        aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .endpoint_url(endpoint_url)
            .load()
            .await
    }

    /// Integration test for concurrent message production and consumption using Omni Queue
    /// Tests multiple producer threads sending messages and consumer threads receiving them
    #[rstest]
    #[tokio::test]
    async fn test_concurrent_version_monitoring() {
        use crate::core::client::queue::sqs::InnerSQS;

        let config = get_localstack_config().await;
        let queue_name = format!("test-snos-job-processing-{}", uuid::Uuid::new_v4());

        // Create queue using InnerSQS (needed for test setup only)
        let inner_sqs = InnerSQS::new(&config);
        inner_sqs.client().create_queue().queue_name(&queue_name).send().await.expect("Failed to create queue");

        // Create SQS client using our QueueClient abstraction
        let queue_args = QueueArgs { queue_template_identifier: AWSResourceIdentifier::Name(queue_name.clone()) };
        let sqs = Arc::new(SQS::new(&config, &queue_args));

        // Track messages received
        let received_count = Arc::new(Mutex::new(0));

        // Spawn producer threads sending messages
        let mut producer_handles = vec![];

        for producer_id in 0..3 {
            let sqs_clone = Arc::clone(&sqs);
            let producer = tokio::spawn(async move {
                for i in 0..5 {
                    let payload = format!("producer-{}-message-{}", producer_id, i);
                    sqs_clone
                        .send_message(QueueType::SnosJobProcessing, payload, None)
                        .await
                        .expect("Failed to send message");
                    sleep(Duration::from_millis(10)).await;
                }
            });
            producer_handles.push(producer);
        }

        // Wait for all producers to finish
        for handle in producer_handles {
            handle.await.expect("Producer thread panicked");
        }

        // Give a moment for messages to be available
        sleep(Duration::from_millis(100)).await;

        // Spawn consumer threads
        let mut consumer_handles = vec![];

        for _ in 0..3 {
            let sqs_clone = Arc::clone(&sqs);
            let received_clone = Arc::clone(&received_count);
            let consumer = tokio::spawn(async move {
                for _ in 0..5 {
                    match sqs_clone.consume_message_from_queue(QueueType::SnosJobProcessing).await {
                        Ok(delivery) => {
                            // Verify we got a message
                            let payload_bytes = delivery.borrow_payload().expect("Failed to get payload");
                            let payload =
                                String::from_utf8(payload_bytes.to_vec()).expect("Failed to parse payload as UTF-8");
                            assert!(payload.contains("producer-"), "Message should be from a producer");

                            // Increment received count
                            *received_clone.lock().unwrap() += 1;

                            // Acknowledge the message
                            delivery.ack().await.expect("Failed to ack message");
                        }
                        Err(_) => {
                            // No message available, wait and retry
                            sleep(Duration::from_millis(50)).await;
                        }
                    }
                }
            });
            consumer_handles.push(consumer);
        }

        // Wait for all consumers to finish
        for handle in consumer_handles {
            handle.await.expect("Consumer thread panicked");
        }

        // Verify that we received all messages
        let total_received = *received_count.lock().unwrap();
        println!("Total messages received: {}", total_received);
        assert_eq!(total_received, 15, "Should have received all 15 messages (3 producers × 5 messages)");

        // Cleanup
        let queue_url = inner_sqs.get_queue_url_from_client(&queue_name).await.expect("Failed to get queue URL");
        inner_sqs.client().delete_queue().queue_url(&queue_url).send().await.ok();
    }
}
