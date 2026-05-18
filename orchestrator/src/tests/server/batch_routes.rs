#![allow(clippy::await_holding_lock)]

use std::net::SocketAddr;
use std::sync::Arc;

use chrono::{Duration, SubsecRound, Utc};
use hyper::{Body, Request};
use rstest::*;
use starknet_api::execution_resources::GasAmount;

use crate::core::config::Config;
use crate::server::types::{ApiResponse, BatchDetailsResponse};
use crate::tests::config::{ConfigType, TestConfigBuilder};
use crate::tests::utils::{build_batch, build_snos_batch};
use crate::types::batch::{AggregatorBatchStatus, SnosBatchStatus};

#[fixture]
async fn setup_batches_server() -> (SocketAddr, Arc<Config>) {
    dotenvy::from_filename_override("../.env.test").expect("Failed to load the .env.test file");

    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_api_server(ConfigType::Actual)
        .build()
        .await;

    let addr = services.api_server_address.unwrap();
    let config = services.config;
    (addr, config)
}

#[rstest]
#[tokio::test]
async fn test_get_snos_batch_details_with_parent_aggregator(#[future] setup_batches_server: (SocketAddr, Arc<Config>)) {
    let (addr, config) = setup_batches_server.await;
    let now = Utc::now().round_subsecs(0);

    let mut aggregator_batch = build_batch(11, 100, 119);
    aggregator_batch.status = AggregatorBatchStatus::ReadyForStateUpdate;
    aggregator_batch.blob_len = 777;
    aggregator_batch.created_at = now - Duration::minutes(12);
    aggregator_batch.updated_at = now - Duration::minutes(2);

    let mut snos_batch = build_snos_batch(21, Some(aggregator_batch.index), 100);
    snos_batch.end_block = 109;
    snos_batch.num_blocks = 10;
    snos_batch.status = SnosBatchStatus::Completed;
    snos_batch.created_at = now - Duration::minutes(11);
    snos_batch.updated_at = now - Duration::minutes(3);
    snos_batch.builtin_weights.state_diff_size = 333;
    snos_batch.builtin_weights.sierra_gas = GasAmount(444);
    snos_batch.builtin_weights.proving_gas = GasAmount(555);

    config.database().create_aggregator_batch(aggregator_batch.clone()).await.unwrap();
    config.database().create_snos_batch(snos_batch.clone()).await.unwrap();

    let client = hyper::Client::new();
    let response = client
        .request(
            Request::builder()
                .uri(format!("http://{}/batches/snos/{}/details", addr, snos_batch.index))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body_bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
    let response_body: ApiResponse<BatchDetailsResponse> = serde_json::from_slice(&body_bytes).unwrap();

    assert!(response_body.success);
    assert_eq!(
        response_body.message,
        Some(format!("Successfully fetched details for SNOS batch {}", snos_batch.index))
    );

    let data = response_body.data.expect("missing batch details payload");
    assert_eq!(data.snos_batch.index, snos_batch.index);
    assert_eq!(data.snos_batch.aggregator_batch_index, Some(aggregator_batch.index));
    assert_eq!(data.snos_batch.start_block, snos_batch.start_block);
    assert_eq!(data.snos_batch.end_block, snos_batch.end_block);
    assert_eq!(data.snos_batch.status, snos_batch.status);
    assert_eq!(data.snos_batch.created_at, snos_batch.created_at);
    assert_eq!(data.snos_batch.updated_at, snos_batch.updated_at);
    assert_eq!(data.snos_batch.metrics.state_diff_size, 333);
    assert_eq!(data.snos_batch.metrics.sierra_gas, 444);
    assert_eq!(data.snos_batch.metrics.proving_gas, 555);

    let aggregator = data.aggregator_batch.expect("missing aggregator batch");
    assert_eq!(aggregator.index, aggregator_batch.index);
    assert_eq!(aggregator.start_block, aggregator_batch.start_block);
    assert_eq!(aggregator.end_block, aggregator_batch.end_block);
    assert_eq!(aggregator.status, aggregator_batch.status);
    assert_eq!(aggregator.created_at, aggregator_batch.created_at);
    assert_eq!(aggregator.updated_at, aggregator_batch.updated_at);
    assert_eq!(aggregator.blob_len, aggregator_batch.blob_len);
}

#[rstest]
#[tokio::test]
async fn test_get_snos_batch_details_without_parent_aggregator(
    #[future] setup_batches_server: (SocketAddr, Arc<Config>),
) {
    let (addr, config) = setup_batches_server.await;

    let mut snos_batch = build_snos_batch(52, None, 300);
    snos_batch.end_block = 309;
    snos_batch.num_blocks = 10;
    snos_batch.status = SnosBatchStatus::Closed;
    snos_batch.builtin_weights.state_diff_size = 901;
    snos_batch.builtin_weights.sierra_gas = GasAmount(902);
    snos_batch.builtin_weights.proving_gas = GasAmount(903);

    config.database().create_snos_batch(snos_batch.clone()).await.unwrap();

    let client = hyper::Client::new();
    let response = client
        .request(
            Request::builder()
                .uri(format!("http://{}/batches/snos/{}/details", addr, snos_batch.index))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body_bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
    let response_body: ApiResponse<BatchDetailsResponse> = serde_json::from_slice(&body_bytes).unwrap();

    let data = response_body.data.expect("missing batch details payload");
    assert_eq!(data.snos_batch.index, snos_batch.index);
    assert_eq!(data.snos_batch.metrics.state_diff_size, 901);
    assert_eq!(data.snos_batch.metrics.sierra_gas, 902);
    assert_eq!(data.snos_batch.metrics.proving_gas, 903);
    assert!(data.aggregator_batch.is_none());
}
