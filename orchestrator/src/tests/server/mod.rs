pub mod admin_routes;
pub mod batch_routes;
pub mod block_routes;
pub mod job_routes;
pub mod jobs_by_status;
use crate::tests::config::{ConfigType, TestConfigBuilder};
use crate::worker::initialize_worker;
use rstest::*;
use tokio_util::sync::CancellationToken;

#[rstest]
#[tokio::test]
async fn test_health_endpoint() {
    dotenvy::from_filename_override("../.env.test").expect("Failed to load the .env.test file");

    let services = TestConfigBuilder::new().configure_api_server(ConfigType::Actual).build().await;

    let addr = services.api_server_address.unwrap();
    let response = reqwest::get(format!("http://{}/health", addr)).await.unwrap();

    assert_eq!(response.status(), 200);

    assert_eq!(response.text().await.unwrap().len(), 2);
}

/// This test case will make sure that the consumers are initialized correctly.
/// and not validate on the queue client data validation and other think to be done wrt to queue business logic.
/// Reason to add timeout login we have try_join_all in this code block which will wait for all the consumers to be Completed
/// [which is not going to happen anytime soon].
/// Better is to wait for some time and understand that the consumers are initialized correctly.
#[rstest]
#[tokio::test]
async fn test_init_consumer() {
    let services = TestConfigBuilder::new().configure_queue_client(ConfigType::Actual).build().await;

    let result = initialize_worker(services.config, CancellationToken::new()).await;

    assert!(result.is_ok(), "Failed to initialize consumers: {:?}", result.err());
}
