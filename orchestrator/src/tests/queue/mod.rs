use rstest::*;

use crate::types::constant::generate_version_string;
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
    let version = generate_version_string();

    // Version should follow the format: starknet-X.X.X::orchestrator-X.X.X
    assert!(version.contains("starknet-"));
    assert!(version.contains("::orchestrator-"));
    assert!(version.split("::").count() == 2);
}

/// Test message attribute building
#[rstest]
#[tokio::test]
async fn test_message_attribute_creation() {
    let version = generate_version_string();

    // Test that MessageAttributeValue can be built successfully
    let attribute_result = MessageAttributeValue::builder()
        .data_type("String")
        .string_value(&version)
        .build();

    assert!(attribute_result.is_ok());

    let attribute = attribute_result.unwrap();
    assert_eq!(attribute.string_value(), Some(version.as_str()));
    assert_eq!(attribute.data_type(), "String");
}

/// Test version matching logic
#[rstest]
#[tokio::test]
async fn test_version_matching() {
    let our_version = generate_version_string();
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

    /// Integration test for full message flow (requires localstack or AWS)
    #[rstest]
    #[tokio::test]
    #[ignore] // Requires actual queue infrastructure
    async fn test_full_message_flow_with_version() {
        // Setup: Create test queue, send message, receive message
        // Verify version attribute is present and correct
        // This would be implemented with localstack in CI/CD
    }

    /// Integration test for version mismatch handling
    #[rstest]
    #[tokio::test]
    #[ignore] // Requires actual queue infrastructure
    async fn test_version_mismatch_visibility_timeout() {
        // Setup: Send message with different version
        // Consume message
        // Verify visibility timeout is changed to 0
        // Verify message is available again immediately
    }

    /// Integration test for backward compatibility
    #[rstest]
    #[tokio::test]
    #[ignore] // Requires actual queue infrastructure
    async fn test_backward_compatibility_no_version_attribute() {
        // Setup: Send message without version attribute (old message)
        // Consume message
        // Verify message is processed despite missing version
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    /// Test that version strings are consistent
    #[rstest]
    #[tokio::test]
    async fn test_version_consistency() {
        let version1 = generate_version_string();
        let version2 = generate_version_string();

        // Same process should generate same version
        assert_eq!(version1, version2);
    }

    /// Test version string format
    #[rstest]
    #[tokio::test]
    async fn test_version_format() {
        let version = generate_version_string();
        let parts: Vec<&str> = version.split("::").collect();

        assert_eq!(parts.len(), 2, "Version should have exactly 2 parts separated by '::'");
        assert!(parts[0].starts_with("starknet-"), "First part should start with 'starknet-'");
        assert!(parts[1].starts_with("orchestrator-"), "Second part should start with 'orchestrator-'");
    }
}
