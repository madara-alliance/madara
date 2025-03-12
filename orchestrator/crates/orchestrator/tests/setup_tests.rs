use aws_config::SdkConfig;
use orchestrator::core::cloud::CloudProvider;
use orchestrator::resource::aws::s3::SSS;
use orchestrator::resource::aws::sqs::SQS;
use orchestrator::resource::setup::{ResourceFactory, ResourceType};

// Create a simple mock CloudProvider for testing
fn mock_cloud_provider() -> CloudProvider {
    // Create a minimal SdkConfig
    let config = SdkConfig::builder().build();
    CloudProvider::AWS(Box::new(config))
}

#[tokio::test]
async fn test_resource_factory_s3_creation() {
    // Create a factory
    let factory = ResourceFactory::new();

    // Create a mock cloud provider
    let provider = mock_cloud_provider();

    // Create an S3 resource
    let resource = factory.create_resource(ResourceType::S3, provider).await;

    // Verify the resource was created successfully
    assert!(resource.is_ok(), "Failed to create S3 resource: {:?}", resource.err());

    // Check if we can downcast to the correct type
    let wrapper = resource.unwrap();
    assert_eq!(wrapper.get_type(), &ResourceType::S3);
    assert!(wrapper.downcast_ref::<SSS>().is_some(), "Failed to downcast to SSS");
}

#[tokio::test]
async fn test_resource_factory_from_str() {
    // Create a factory
    let factory = ResourceFactory::new();

    // Create a mock cloud provider
    let provider = mock_cloud_provider();

    // Create resources from string types
    let s3_resource = factory.create_resource_from_str("s3", provider.clone()).await;
    assert!(s3_resource.is_ok(), "Failed to create S3 resource from string: {:?}", s3_resource.err());

    let s3_wrapper = s3_resource.unwrap();
    assert_eq!(s3_wrapper.get_type(), &ResourceType::S3);
    assert!(s3_wrapper.downcast_ref::<SSS>().is_some(), "Failed to downcast to SSS");

    let sqs_resource = factory.create_resource_from_str("sqs", provider.clone()).await;
    assert!(sqs_resource.is_ok(), "Failed to create SQS resource from string: {:?}", sqs_resource.err());

    let sqs_wrapper = sqs_resource.unwrap();
    assert_eq!(sqs_wrapper.get_type(), &ResourceType::SQS);
    assert!(sqs_wrapper.downcast_ref::<SQS>().is_some(), "Failed to downcast to SQS");

    // Test invalid resource type
    let invalid_resource = factory.create_resource_from_str("invalid", provider).await;
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
