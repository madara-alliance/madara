#![allow(clippy::await_holding_lock)]

use std::net::SocketAddr;
use std::sync::Arc;

use chrono::{Duration, SubsecRound, Utc};
use hyper::{Body, Request};
use rstest::*;
use starknet_api::execution_resources::GasAmount;

use crate::core::config::Config;
use crate::server::types::{AggregatorBatchListResponse, ApiResponse, SnosBatchListResponse};
use crate::tests::config::{ConfigType, TestConfigBuilder};
use crate::tests::utils::{build_batch, build_snos_batch};
use crate::types::batch::{AggregatorBatchStatus, SnosBatchStatus};

#[fixture]
async fn setup_batches_server() -> (SocketAddr, Arc<Config>) {
    dotenvy::from_filename_override("../.env.test").expect("Failed to load the .env.test file");
    if std::env::var("MADARA_ORCHESTRATOR_ETHEREUM_SETTLEMENT_RPC_URL").is_err() {
        std::env::set_var("MADARA_ORCHESTRATOR_ETHEREUM_SETTLEMENT_RPC_URL", "http://localhost:8545");
    }

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
async fn test_query_snos_batches_by_index(#[future] setup_batches_server: (SocketAddr, Arc<Config>)) {
    let (addr, config) = setup_batches_server.await;
    let now = Utc::now().round_subsecs(0);

    let mut aggregator_batch = build_batch(11, 100, 119);
    aggregator_batch.status = AggregatorBatchStatus::ReadyForStateUpdate;
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
                .uri(format!("http://{}/batches/snos?index={}", addr, snos_batch.index))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body_bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
    let response_body: ApiResponse<SnosBatchListResponse> = serde_json::from_slice(&body_bytes).unwrap();

    assert!(response_body.success);
    let batches = response_body.data.expect("missing snos batches payload").batches;
    assert_eq!(batches.len(), 1);

    let batch = &batches[0];
    assert_eq!(batch.batch.index, snos_batch.index);
    assert_eq!(batch.batch.aggregator_batch_index, Some(aggregator_batch.index));
    assert_eq!(batch.batch.start_block, snos_batch.start_block);
    assert_eq!(batch.batch.end_block, snos_batch.end_block);
    assert_eq!(batch.batch.status, snos_batch.status);
    assert_eq!(batch.metrics.state_diff_size, 333);
    assert_eq!(batch.metrics.sierra_gas, 444);
    assert_eq!(batch.metrics.proving_gas, 555);
}

#[rstest]
#[tokio::test]
async fn test_query_aggregator_batches_closed_filter_returns_latest_closed(
    #[future] setup_batches_server: (SocketAddr, Arc<Config>),
) {
    let (addr, config) = setup_batches_server.await;

    let mut older_closed_batch = build_batch(11, 100, 119);
    older_closed_batch.status = AggregatorBatchStatus::Closed;
    older_closed_batch.blob_len = 777;

    let mut latest_closed_batch = build_batch(12, 120, 139);
    latest_closed_batch.status = AggregatorBatchStatus::ReadyForStateUpdate;
    latest_closed_batch.blob_len = 888;

    let mut open_batch = build_batch(13, 140, 149);
    open_batch.status = AggregatorBatchStatus::Open;
    open_batch.blob_len = 999;

    config.database().create_aggregator_batch(older_closed_batch.clone()).await.unwrap();
    config.database().create_aggregator_batch(latest_closed_batch.clone()).await.unwrap();
    config.database().create_aggregator_batch(open_batch.clone()).await.unwrap();

    let client = hyper::Client::new();
    let response = client
        .request(
            Request::builder()
                .uri(format!("http://{}/batches/aggregator?status=closed&limit=1&sort=desc", addr))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body_bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
    let response_body: ApiResponse<AggregatorBatchListResponse> = serde_json::from_slice(&body_bytes).unwrap();

    assert!(response_body.success);
    let batches = response_body.data.expect("missing aggregator batches payload").batches;
    assert_eq!(batches.len(), 1);

    let batch = &batches[0];
    assert_eq!(batch.batch.index, latest_closed_batch.index);
    assert_eq!(batch.batch.start_block, latest_closed_batch.start_block);
    assert_eq!(batch.batch.end_block, latest_closed_batch.end_block);
    assert_eq!(batch.batch.status, latest_closed_batch.status);
    assert_eq!(batch.blob_len, latest_closed_batch.blob_len);
}
