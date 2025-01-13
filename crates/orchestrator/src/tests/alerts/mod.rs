use std::time::Duration;

use aws_sdk_sqs::types::QueueAttributeName::QueueArn;
use rstest::rstest;
use tokio::time::sleep;
use utils::env_utils::get_env_var_or_panic;

use crate::tests::common::{get_sns_client, get_sqs_client};
use crate::tests::config::{ConfigType, TestConfigBuilder};

pub const SNS_ALERT_TEST_QUEUE: &str = "orchestrator_sns_alert_testing_queue";

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

    // subscribing the queue with the alerts
    sns_client
        .subscribe()
        .topic_arn(get_env_var_or_panic("MADARA_ORCHESTRATOR_AWS_SNS_ARN").as_str())
        .protocol("sqs")
        .endpoint(queue_arn)
        .send()
        .await
        .unwrap();

    let message_to_send = "Hello World :)";

    // Getting sns client from the module
    let alerts_client = services.config.alerts();
    // Sending the alert message
    alerts_client.send_alert_message(message_to_send.to_string()).await.unwrap();

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
