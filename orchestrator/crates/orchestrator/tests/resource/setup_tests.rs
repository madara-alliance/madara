use orchestrator::core::cloud::CloudProvider;
use orchestrator::resource::setup::{ResourceFactory, ResourceType};

#[tokio::test]
async fn test_resource_factory_s3_creation() {
    // Create a factory
    let factory = ResourceFactory::new();

    // Create an S3 resource
    let resource = factory.create_resource(ResourceType::S3, CloudProvider::AWS).await;

    // Verify the resource was created successfully
    assert!(resource.is_ok(), "Failed to create S3 resource: {:?}", resource.err());
}

#[tokio::test]
async fn test_resource_factory_from_str() {
    // Create a factory
    let factory = ResourceFactory::new();

    // Create resources from string types
    let s3_resource = factory.create_resource_from_str("s3", CloudProvider::AWS).await;
    assert!(s3_resource.is_ok(), "Failed to create S3 resource from string: {:?}", s3_resource.err());

    let sqs_resource = factory.create_resource_from_str("sqs", CloudProvider::AWS).await;
    assert!(sqs_resource.is_ok(), "Failed to create SQS resource from string: {:?}", sqs_resource.err());

    // Test invalid resource type
    let invalid_resource = factory.create_resource_from_str("invalid", CloudProvider::AWS).await;
    assert!(invalid_resource.is_err(), "Expected error for invalid resource type");
}

#[tokio::test]
async fn test_resource_type_from_str() {
    // Test valid resource types
    assert_eq!(ResourceType::from_str("s3").unwrap(), ResourceType::S3);
    assert_eq!(ResourceType::from_str("sqs").unwrap(), ResourceType::SQS);

    // Test case insensitivity
    assert_eq!(ResourceType::from_str("S3").unwrap(), ResourceType::S3);
    assert_eq!(ResourceType::from_str("SQS").unwrap(), ResourceType::SQS);

    // Test invalid resource type
    assert!(ResourceType::from_str("invalid").is_none());
}
