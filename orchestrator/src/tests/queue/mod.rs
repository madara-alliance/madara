use rstest::*;

use crate::types::constant::get_version_string;
use crate::types::queue::QueueType;
use aws_sdk_sqs::types::MessageAttributeValue;

/// Test that send_message adds the OrchestratorVersion message attribute
#[rstest]
#[tokio::test]
#[ignore] // Requires AWS credentials and actual queue
async fn test_send_message_with_version_attribute() {
    // This test would require actual AWS credentials and queue setup
    // Keeping as documentation of expected behavior
}

/// Test version string generation
#[rstest]
#[tokio::test]
async fn test_version_string_generation() {
    let version = get_version_string();

    // Version should follow the format: starknet-X.X.X::orchestrator-X.X.X
    assert!(version.contains("starknet-"));
    assert!(version.contains("::orchestrator-"));
    assert!(version.split("::").count() == 2);
}

/// Test message attribute building
#[rstest]
#[tokio::test]
async fn test_message_attribute_creation() {
    let version = get_version_string();

    // Test that MessageAttributeValue can be built successfully
    let attribute_result = MessageAttributeValue::builder().data_type("String").string_value(&version).build();

    assert!(attribute_result.is_ok());

    let attribute = attribute_result.unwrap();
    assert_eq!(attribute.string_value(), Some(version.as_str()));
    assert_eq!(attribute.data_type(), "String");
}

/// Test version matching logic
#[rstest]
#[tokio::test]
async fn test_version_matching() {
    let our_version = get_version_string();
    let same_version = our_version.clone();
    let different_version = "starknet-0.0.0::orchestrator-0.0.0".to_string();

    // Same version should match
    assert_eq!(our_version, same_version);

    // Different version should not match
    assert_ne!(our_version, different_version);
}

/// Test queue type enumeration
#[rstest]
#[tokio::test]
async fn test_queue_types() {
    use strum::IntoEnumIterator;

    // Ensure all queue types can be iterated
    let queue_types: Vec<QueueType> = QueueType::iter().collect();
    assert!(!queue_types.is_empty());

    // Each queue type should have a string representation
    for queue_type in queue_types {
        let queue_str = queue_type.to_string();
        assert!(!queue_str.is_empty());
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use aws_config::{BehaviorVersion, Region};
    use aws_sdk_sqs::types::MessageAttributeValue as AwsMessageAttributeValue;

    /// Helper to create localstack AWS config
    async fn get_localstack_config() -> aws_config::SdkConfig {
        aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .endpoint_url("http://localhost:4566")
            .load()
            .await
    }

    /// Integration test for full message flow (requires localstack or AWS)
    #[rstest]
    #[tokio::test]
    #[ignore] // Run with: cargo test --lib tests::queue::integration_tests -- --ignored --test-threads=1
    async fn test_full_message_flow_with_version() {
        let config = get_localstack_config().await;
        let queue_name = format!("test-queue-{}", uuid::Uuid::new_v4());

        // Create test queue
        let sqs_client = aws_sdk_sqs::Client::new(&config);
        sqs_client.create_queue().queue_name(&queue_name).send().await.expect("Failed to create queue");

        let queue_url = sqs_client
            .get_queue_url()
            .queue_name(&queue_name)
            .send()
            .await
            .expect("Failed to get queue URL")
            .queue_url()
            .unwrap()
            .to_string();

        // Send message with version attribute
        let test_payload = "test message content";
        let version = get_version_string();

        let version_attr = AwsMessageAttributeValue::builder()
            .data_type("String")
            .string_value(&version)
            .build()
            .expect("Failed to build attribute");

        sqs_client
            .send_message()
            .queue_url(&queue_url)
            .message_body(test_payload)
            .message_attributes("OrchestratorVersion", version_attr)
            .send()
            .await
            .expect("Failed to send message");

        // Receive and verify message
        let receive_result = sqs_client
            .receive_message()
            .queue_url(&queue_url)
            .message_attribute_names("OrchestratorVersion")
            .send()
            .await
            .expect("Failed to receive message");

        let messages = receive_result.messages();
        assert!(!messages.is_empty(), "No messages received");
        assert_eq!(messages.len(), 1, "Expected exactly one message");

        let message = &messages[0];
        assert_eq!(message.body(), Some(test_payload));

        // Verify version attribute
        let attrs = message.message_attributes();
        assert!(attrs.is_some(), "No attributes");
        let version_attr = attrs.unwrap().get("OrchestratorVersion");
        assert!(version_attr.is_some(), "Version attribute missing");
        assert_eq!(version_attr.unwrap().string_value(), Some(version.as_str()));

        // Cleanup
        sqs_client.delete_queue().queue_url(&queue_url).send().await.ok();
    }

    /// Integration test for backward compatibility - messages without version attribute
    #[rstest]
    #[tokio::test]
    #[ignore] // Run with: cargo test --lib tests::queue::integration_tests -- --ignored --test-threads=1
    async fn test_backward_compatibility_no_version_attribute() {
        let config = get_localstack_config().await;
        let queue_name = format!("test-queue-{}", uuid::Uuid::new_v4());

        // Create test queue
        let sqs_client = aws_sdk_sqs::Client::new(&config);
        sqs_client.create_queue().queue_name(&queue_name).send().await.expect("Failed to create queue");

        let queue_url = sqs_client
            .get_queue_url()
            .queue_name(&queue_name)
            .send()
            .await
            .expect("Failed to get queue URL")
            .queue_url()
            .unwrap()
            .to_string();

        // Send message WITHOUT version attribute (simulating legacy message)
        let test_payload = "legacy message";

        sqs_client
            .send_message()
            .queue_url(&queue_url)
            .message_body(test_payload)
            .send()
            .await
            .expect("Failed to send message");

        // Receive and verify message is processed
        let receive_result = sqs_client
            .receive_message()
            .queue_url(&queue_url)
            .message_attribute_names("OrchestratorVersion")
            .send()
            .await
            .expect("Failed to receive message");

        let messages = receive_result.messages();
        assert!(!messages.is_empty(), "No messages received");
        assert_eq!(messages.len(), 1, "Expected exactly one message");

        let message = &messages[0];
        assert_eq!(message.body(), Some(test_payload));

        // Verify NO version attribute (backward compatibility)
        let attrs = message.message_attributes();
        assert!(
            attrs.is_none() || !attrs.unwrap().contains_key("OrchestratorVersion"),
            "Legacy message should not have version attribute"
        );

        // Cleanup
        sqs_client.delete_queue().queue_url(&queue_url).send().await.ok();
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    /// Test that version strings are consistent
    #[rstest]
    #[tokio::test]
    async fn test_version_consistency() {
        let version1 = get_version_string();
        let version2 = get_version_string();

        // Same process should generate same version
        assert_eq!(version1, version2);
    }

    /// Test version string format
    #[rstest]
    #[tokio::test]
    async fn test_version_format() {
        let version = get_version_string();
        let parts: Vec<&str> = version.split("::").collect();

        assert_eq!(parts.len(), 2, "Version should have exactly 2 parts separated by '::'");
        assert!(parts[0].starts_with("starknet-"), "First part should start with 'starknet-'");
        assert!(parts[1].starts_with("orchestrator-"), "Second part should start with 'orchestrator-'");
    }
}
