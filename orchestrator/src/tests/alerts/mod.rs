use crate::core::client::SNS;
use crate::tests::common::{get_sns_client, get_sqs_client};
use crate::tests::config::{ConfigType, TestConfigBuilder};
use crate::types::params::{AWSResourceIdentifier, AlertArgs};
use aws_sdk_sqs::types::QueueAttributeName::QueueArn;
use orchestrator_utils::env_utils::get_env_var_or_panic;
use rstest::rstest;
use std::time::Duration;
use tokio::time::sleep;

pub const SNS_ALERT_TEST_QUEUE: &str = "orchestrator_sns_alert_testing_queue";

/// This test is used to test the SNS alert subscription and message sending functionality.
/// It creates a new SQS queue, subscribes it to the SNS topic, and sends a test message.
/// It then checks if the message is received in the SQS queue.
#[rstest]
#[tokio::test]
async fn sns_alert_subscribe_to_topic_receive_alert_works() {
    let services = TestConfigBuilder::new().configure_alerts(ConfigType::Actual).build().await;

    let sqs_client = get_sqs_client(services.provider_config.clone()).await;
    let queue = sqs_client.create_queue().queue_name(SNS_ALERT_TEST_QUEUE).send().await.unwrap();
    let queue_url = queue.queue_url().unwrap();

    let sns_client = get_sns_client(services.provider_config.get_aws_client_or_panic()).await;

    let queue_attributes =
        sqs_client.get_queue_attributes().queue_url(queue_url).attribute_names(QueueArn).send().await.unwrap();

    let queue_arn = queue_attributes.attributes().unwrap().get(&QueueArn).unwrap();

    let alert_identifier =
        AWSResourceIdentifier::Name(get_env_var_or_panic("MADARA_ORCHESTRATOR_AWS_SNS_TOPIC_IDENTIFIER"));
    let alert_config = AlertArgs { alert_identifier };
    let sns = SNS::new(services.provider_config.get_aws_client_or_panic(), &alert_config);

    let sns_arn = sns.get_topic_arn().await.unwrap();

    sns_client.subscribe().topic_arn(sns_arn).protocol("sqs").endpoint(queue_arn).send().await.unwrap();

    let message_to_send = "Hello World :)";

    // Getting sns client from the module
    let alerts_client = services.config.alerts();
    // Sending the alert message
    alerts_client.send_message(message_to_send.to_string()).await.unwrap();

    sleep(Duration::from_secs(5)).await;

    // Checking the queue for message
    let receive_message_result = sqs_client
        .receive_message()
        .queue_url(queue_url)
        .max_number_of_messages(1)
        .send()
        .await
        .unwrap()
        .messages
        .unwrap();

    assert_eq!(receive_message_result.len(), 1, "Alert message length assertion failed");
    assert!(receive_message_result[0].body.clone().unwrap().contains(message_to_send));
}
