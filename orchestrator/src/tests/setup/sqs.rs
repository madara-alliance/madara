use crate::core::client::queue::sqs::InnerSQS;
use crate::core::cloud::CloudProvider;
use crate::core::traits::resource::Resource;
use crate::tests::common::get_sqs_client;
use crate::types::params::{AWSResourceIdentifier, QueueArgs, ARN};
use crate::types::queue_control::QUEUES;
use crate::types::Layer;
use aws_config::{BehaviorVersion, Region};
use aws_sdk_sqs::types::QueueAttributeName;
use orchestrator_utils::env_utils::get_env_var_or_panic;
use rstest::*;
use std::collections::HashMap;
use std::sync::Arc;

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

/// Fixture to create a cloud provider with localstack config
#[fixture]
async fn cloud_provider(#[future] localstack_config: aws_config::SdkConfig) -> Arc<CloudProvider> {
    let config = localstack_config.await;
    Arc::new(CloudProvider::AWS(Box::new(config)))
}

/// Fixture to create queue args with a unique queue template
#[fixture]
fn queue_args() -> QueueArgs {
    let uuid_prefix = &uuid::Uuid::new_v4().to_string()[0..4];
    let queue_template = format!("test-{}-{{}}_queue", uuid_prefix);
    QueueArgs { queue_template_identifier: AWSResourceIdentifier::Name(queue_template) }
}

/// Helper function to cleanup queues for a specific test (only deletes queues matching the queue_args identifier)
async fn cleanup_queues_for_test(provider_config: Arc<CloudProvider>, queue_args: &QueueArgs) {
    use crate::tests::common::cleanup_queues;
    let _ = cleanup_queues(provider_config, queue_args).await;
}

/// Helper function to cleanup all test queues (use with caution in parallel tests)
/// This is kept for backward compatibility but should be avoided in parallel test scenarios
async fn cleanup_queues(provider_config: Arc<CloudProvider>) {
    let sqs_client = get_sqs_client(provider_config).await;
    let list_queues_output = sqs_client.list_queues().send().await.expect("Failed to list queues");
    let queue_urls = list_queues_output.queue_urls();

    for queue_url in queue_urls {
        if queue_url.contains("test-") {
            sqs_client.delete_queue().queue_url(queue_url).send().await.ok();
        }
    }
}

/// Helper function to verify queue setup for a given layer
async fn verify_queue_setup(inner_sqs: &InnerSQS, layer: &Layer, queue_args: &QueueArgs) -> color_eyre::Result<()> {
    let queue_template_name = match &queue_args.queue_template_identifier {
        AWSResourceIdentifier::Name(name) => name,
        AWSResourceIdentifier::ARN(arn) => &arn.resource,
    };

    for (queue_type, queue_config) in QUEUES.iter() {
        if !queue_config.supported_layers.contains(layer) {
            continue;
        }

        let queue_name = InnerSQS::get_queue_name_from_type(queue_template_name, queue_type);

        // Verify queue exists
        let queue_url = inner_sqs.get_queue_url_from_client(&queue_name).await?;

        // Verify visibility timeout
        let attributes = inner_sqs
            .client()
            .get_queue_attributes()
            .queue_url(&queue_url)
            .attribute_names(QueueAttributeName::VisibilityTimeout)
            .send()
            .await?;

        let visibility_timeout = attributes
            .attributes()
            .and_then(|attrs| attrs.get(&QueueAttributeName::VisibilityTimeout))
            .and_then(|timeout| timeout.parse::<u32>().ok())
            .expect("Visibility timeout should be set");

        assert_eq!(
            visibility_timeout, queue_config.visibility_timeout,
            "Visibility timeout mismatch for queue {}",
            queue_name
        );

        // Verify DLQ configuration if present
        if let Some(dlq_config) = &queue_config.dlq_config {
            let dlq_name = InnerSQS::get_queue_name_from_type(queue_template_name, &dlq_config.dlq_name);

            // Verify DLQ exists
            let dlq_url = inner_sqs.get_queue_url_from_client(&dlq_name).await;
            assert!(dlq_url.is_ok(), "DLQ {} should exist for queue {}", dlq_name, queue_name);

            // Verify redrive policy
            let attributes = inner_sqs
                .client()
                .get_queue_attributes()
                .queue_url(&queue_url)
                .attribute_names(QueueAttributeName::RedrivePolicy)
                .send()
                .await?;

            let redrive_policy = attributes
                .attributes()
                .and_then(|attrs| attrs.get(&QueueAttributeName::RedrivePolicy))
                .expect("Redrive policy should be set");

            // Parse and verify redrive policy
            let policy_json: HashMap<String, serde_json::Value> =
                serde_json::from_str(redrive_policy).expect("Should parse redrive policy JSON");

            assert!(
                policy_json.contains_key("deadLetterTargetArn"),
                "Redrive policy should contain deadLetterTargetArn"
            );

            let max_receive_count = policy_json
                .get("maxReceiveCount")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u32>().ok())
                .expect("Should have maxReceiveCount in policy");

            assert_eq!(
                max_receive_count, dlq_config.max_receive_count,
                "Max receive count mismatch for queue {}",
                queue_name
            );
        }

        println!("Queue properly setup: {}", queue_name);
    }

    Ok(())
}

/// Test setup with Name identifier for both L2 and L3 layers
/// Verifies: queue creation, DLQs, visibility timeout, max receive count, is_ready_to_use
#[rstest]
#[case(Layer::L2)]
#[case(Layer::L3)]
#[tokio::test]
async fn test_setup_with_name_identifier(
    #[future] cloud_provider: Arc<CloudProvider>,
    queue_args: QueueArgs,
    #[case] layer: Layer,
) -> color_eyre::Result<()> {
    let provider = cloud_provider.await;
    // Use selective cleanup instead of deleting all test queues
    // This prevents conflicts with other parallel tests
    cleanup_queues_for_test(provider.clone(), &queue_args).await;

    let inner_sqs = InnerSQS::create_setup(provider.clone()).await?;

    // Verify queues are not ready before setup
    let ready_before = inner_sqs.is_ready_to_use(&layer, &queue_args).await?;
    assert!(!ready_before, "Queues should not be ready before setup");

    // Run setup
    inner_sqs.setup(&layer, queue_args.clone()).await?;
    println!("✓ Setup completed for {:?} layer", layer);

    // Verify all queues, DLQs, timeouts, and max counts
    verify_queue_setup(&inner_sqs, &layer, &queue_args).await?;

    // Verify queues are ready after setup
    let ready_after = inner_sqs.is_ready_to_use(&layer, &queue_args).await?;
    assert!(ready_after, "Queues should be ready after setup");
    println!("✓ is_ready_to_use returns true for {:?} layer", layer);

    cleanup_queues_for_test(provider.clone(), &queue_args).await;
    Ok(())
}

/// Test setup with ARN identifier for both L2 and L3 layers
/// Verifies: queue creation, DLQs, visibility timeout, max receive count, is_ready_to_use
#[rstest]
#[case(Layer::L2)]
#[case(Layer::L3)]
#[tokio::test]
async fn test_setup_with_arn_identifier(
    #[future] cloud_provider: Arc<CloudProvider>,
    #[case] layer: Layer,
) -> color_eyre::Result<()> {
    let provider = cloud_provider.await;
    // For ARN test, we'll create queues with a unique template, so we don't need to cleanup first
    // The unique UUID prefix ensures no conflicts with other parallel tests
    let inner_sqs = InnerSQS::create_setup(provider.clone()).await?;

    let uuid_prefix = &uuid::Uuid::new_v4().to_string()[0..4];
    let queue_template = format!("test-arn-{}-{{}}_queue", uuid_prefix);

    // Create test ARN identifier
    let arn = ARN {
        partition: "aws".to_string(),
        service: "sqs".to_string(),
        region: "us-east-1".to_string(),
        account_id: "000000000000".to_string(),
        resource: queue_template,
    };

    let queue_args_arn = QueueArgs { queue_template_identifier: AWSResourceIdentifier::ARN(arn.clone()) };

    // Verify queues are not ready before setup
    let ready_before = inner_sqs.is_ready_to_use(&layer, &queue_args_arn).await?;
    assert!(!ready_before, "Queues should not be ready before setup");

    // For ARN-based setup, we need to pre-create the queues since ARN setup skips creation
    // First create queues with name identifier
    let name_queue_args = QueueArgs { queue_template_identifier: AWSResourceIdentifier::Name(arn.resource.clone()) };
    inner_sqs.setup(&layer, name_queue_args.clone()).await?;
    println!("✓ Pre-created queues using Name identifier");

    // Now run setup with ARN identifier (should verify existence and skip creation)
    inner_sqs.setup(&layer, queue_args_arn.clone()).await?;
    println!("✓ Setup completed for {:?} layer with ARN identifier", layer);

    // Verify all queues, DLQs, timeouts, and max counts
    verify_queue_setup(&inner_sqs, &layer, &queue_args_arn).await?;

    // Verify queues are ready after setup
    let ready_after = inner_sqs.is_ready_to_use(&layer, &queue_args_arn).await?;
    assert!(ready_after, "Queues should be ready after setup");
    println!("✓ is_ready_to_use returns true for {:?} layer with ARN identifier", layer);

    cleanup_queues_for_test(provider.clone(), &queue_args_arn).await;
    Ok(())
}
