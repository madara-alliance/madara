pub mod job_routes;
use std::io::Read;

use axum::http::StatusCode;
use hyper::body::Buf;
use hyper::{Body, Request};
use rstest::*;

use crate::tests::config::{ConfigType, TestConfigBuilder};
use crate::worker::initialize_worker;

#[rstest]
#[tokio::test]
async fn test_health_endpoint() {
    dotenvy::from_filename_override("../.env.test").expect("Failed to load the .env.test file");

    let services = TestConfigBuilder::new().configure_api_server(ConfigType::Actual).build().await;

    let addr = services.api_server_address.unwrap();
    let client = hyper::Client::new();
    let response = client
        .request(Request::builder().uri(format!("http://{}/health", addr)).body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status().as_str(), StatusCode::OK.as_str());

    let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
    let mut buf = String::new();
    let res = body.reader().read_to_string(&mut buf).unwrap();
    assert_eq!(res, 2);
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

    let result = initialize_worker(services.config, Arc::new(Notify::new())).await;

    assert!(result.is_ok(), "Failed to initialize consumers: {:?}", result.err());
}
