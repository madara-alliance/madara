#![allow(clippy::await_holding_lock)]

use std::net::SocketAddr;
use std::sync::Arc;

use chrono::{Duration, SubsecRound, Utc};
use hyper::{Body, Request};
use rstest::*;
use uuid::Uuid;

use crate::core::config::Config;
use crate::server::types::{ApiResponse, BlockSettlementStatusResponse};
use crate::tests::config::{ConfigType, TestConfigBuilder};
use crate::tests::utils::{build_batch, build_job_item, build_snos_batch};
use crate::types::batch::{AggregatorBatchStatus, SnosBatchStatus};
use crate::types::jobs::external_id::ExternalId;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{
    AggregatorMetadata, CommonMetadata, JobMetadata, JobSpecificMetadata, ProvingInputType, SettlementContext,
    SettlementContextData,
};
use crate::types::jobs::types::{JobStatus, JobType};

#[fixture]
async fn setup_blocks_server() -> (SocketAddr, Arc<Config>) {
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

fn build_aggregator_job(batch_index: u64, status: JobStatus) -> JobItem {
    JobItem {
        id: Uuid::new_v4(),
        internal_id: batch_index,
        job_type: JobType::Aggregator,
        status,
        external_id: ExternalId::String(format!("bucket-{}", batch_index).into_boxed_str()),
        metadata: JobMetadata {
            common: CommonMetadata {
                orchestrator_version: crate::types::constant::ORCHESTRATOR_VERSION.to_string(),
                ..Default::default()
            },
            specific: JobSpecificMetadata::Aggregator(AggregatorMetadata {
                batch_num: batch_index,
                bucket_id: format!("bucket-{}", batch_index),
                ..Default::default()
            }),
        },
        version: 0,
        created_at: Utc::now().round_subsecs(0),
        updated_at: Utc::now().round_subsecs(0),
    }
}

#[rstest]
#[tokio::test]
async fn test_get_block_settlement_status_for_batched_block(#[future] setup_blocks_server: (SocketAddr, Arc<Config>)) {
    let (addr, config) = setup_blocks_server.await;
    let block_number = 105;
    let now = Utc::now().round_subsecs(0);

    let mut aggregator_batch = build_batch(11, 100, 119);
    aggregator_batch.status = AggregatorBatchStatus::ReadyForStateUpdate;

    let mut snos_batch_for_block = build_snos_batch(21, Some(aggregator_batch.index), 100);
    snos_batch_for_block.end_block = 109;
    snos_batch_for_block.num_blocks = 10;
    snos_batch_for_block.status = SnosBatchStatus::Completed;

    let mut other_snos_batch = build_snos_batch(22, Some(aggregator_batch.index), 110);
    other_snos_batch.end_block = 119;
    other_snos_batch.num_blocks = 10;
    other_snos_batch.status = SnosBatchStatus::Completed;

    config.database().create_aggregator_batch(aggregator_batch.clone()).await.unwrap();
    config.database().create_snos_batch(snos_batch_for_block.clone()).await.unwrap();
    config.database().create_snos_batch(other_snos_batch.clone()).await.unwrap();

    let mut snos_job = build_job_item(JobType::SnosRun, JobStatus::Completed, snos_batch_for_block.index);
    if let JobSpecificMetadata::Snos(metadata) = &mut snos_job.metadata.specific {
        metadata.snos_batch_index = snos_batch_for_block.index;
        metadata.start_block = snos_batch_for_block.start_block;
        metadata.end_block = snos_batch_for_block.end_block;
        metadata.num_blocks = snos_batch_for_block.num_blocks;
    } else {
        panic!("Unexpected metadata type for SNOS job");
    }
    snos_job.metadata.common.process_started_at = Some(now - Duration::minutes(12));
    snos_job.metadata.common.process_completed_at = Some(now - Duration::minutes(11));

    let mut proof_job_for_block =
        build_job_item(JobType::ProofCreation, JobStatus::Completed, snos_batch_for_block.index);
    if let JobSpecificMetadata::Proving(metadata) = &mut proof_job_for_block.metadata.specific {
        metadata.block_number = snos_batch_for_block.start_block;
        metadata.input_path = Some(ProvingInputType::CairoPie(format!("{}/proof.cairo", snos_batch_for_block.index)));
    } else {
        panic!("Unexpected metadata type for proof job");
    }
    proof_job_for_block.metadata.common.process_started_at = Some(now - Duration::minutes(10));
    proof_job_for_block.metadata.common.process_completed_at = Some(now - Duration::minutes(9));
    proof_job_for_block.metadata.common.verification_started_at = Some(now - Duration::minutes(8));
    proof_job_for_block.metadata.common.verification_completed_at = Some(now - Duration::minutes(6));

    let mut proof_job_for_other_batch =
        build_job_item(JobType::ProofCreation, JobStatus::Completed, other_snos_batch.index);
    if let JobSpecificMetadata::Proving(metadata) = &mut proof_job_for_other_batch.metadata.specific {
        metadata.block_number = other_snos_batch.start_block;
        metadata.input_path = Some(ProvingInputType::CairoPie(format!("{}/proof.cairo", other_snos_batch.index)));
    } else {
        panic!("Unexpected metadata type for proof job");
    }
    proof_job_for_other_batch.metadata.common.process_started_at = Some(now - Duration::minutes(7));
    proof_job_for_other_batch.metadata.common.process_completed_at = Some(now - Duration::minutes(6));
    proof_job_for_other_batch.metadata.common.verification_started_at = Some(now - Duration::minutes(5));
    proof_job_for_other_batch.metadata.common.verification_completed_at = Some(now - Duration::minutes(4));

    let mut aggregator_job = build_aggregator_job(aggregator_batch.index, JobStatus::PendingVerification);
    aggregator_job.metadata.common.process_started_at = Some(now - Duration::minutes(3));
    aggregator_job.metadata.common.process_completed_at = Some(now - Duration::minutes(2));

    let mut state_transition_job =
        build_job_item(JobType::StateTransition, JobStatus::PendingVerification, aggregator_batch.index);
    if let JobSpecificMetadata::StateUpdate(metadata) = &mut state_transition_job.metadata.specific {
        metadata.context =
            SettlementContext::Batch(SettlementContextData { to_settle: aggregator_batch.index, last_failed: None });
    } else {
        panic!("Unexpected metadata type for state transition job");
    }
    state_transition_job.metadata.common.process_completed_at = Some(now - Duration::minutes(1));
    state_transition_job.metadata.common.verification_started_at = Some(now - Duration::seconds(30));

    config.database().create_job(snos_job.clone()).await.unwrap();
    config.database().create_job(proof_job_for_block.clone()).await.unwrap();
    config.database().create_job(proof_job_for_other_batch.clone()).await.unwrap();
    config.database().create_job(aggregator_job.clone()).await.unwrap();
    config.database().create_job(state_transition_job.clone()).await.unwrap();

    let client = hyper::Client::new();
    let response = client
        .request(
            Request::builder()
                .uri(format!("http://{}/blocks/settlement-status/{}", addr, block_number))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body_bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
    let response_body: ApiResponse<BlockSettlementStatusResponse> = serde_json::from_slice(&body_bytes).unwrap();

    assert!(response_body.success);
    assert_eq!(
        response_body.message,
        Some(format!("Successfully fetched settlement status for block {}", block_number))
    );

    let data = response_body.data.expect("missing settlement status payload");
    assert_eq!(data.block_number, block_number);

    let snos_batch = data.snos_batch.expect("missing snos batch response");
    assert_eq!(snos_batch.index, snos_batch_for_block.index);
    assert_eq!(snos_batch.start_block, snos_batch_for_block.start_block);
    assert_eq!(snos_batch.end_block, snos_batch_for_block.end_block);

    let aggregator_batch_response = data.aggregator_batch.expect("missing aggregator batch response");
    assert_eq!(aggregator_batch_response.index, aggregator_batch.index);
    assert_eq!(aggregator_batch_response.start_block, aggregator_batch.start_block);
    assert_eq!(aggregator_batch_response.end_block, aggregator_batch.end_block);

    assert_eq!(data.block_jobs.len(), 4);
    assert!(data.block_jobs.iter().any(|job| job.id == snos_job.id && job.job_type == JobType::SnosRun));
    assert!(data
        .block_jobs
        .iter()
        .any(|job| job.id == proof_job_for_block.id && job.job_type == JobType::ProofCreation));
    assert!(data.block_jobs.iter().any(|job| job.id == aggregator_job.id && job.job_type == JobType::Aggregator));
    assert!(data
        .block_jobs
        .iter()
        .any(|job| job.id == state_transition_job.id && job.timestamps.process_completed_at.is_some()));

    assert_eq!(data.aggregator_proof_jobs.len(), 2);
    assert!(data.aggregator_proof_jobs.iter().any(|job| job.id == proof_job_for_block.id));
    assert!(data.aggregator_proof_jobs.iter().any(|job| job.id == proof_job_for_other_batch.id));
}
