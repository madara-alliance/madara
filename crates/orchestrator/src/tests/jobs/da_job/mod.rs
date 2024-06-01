use rstest::*;
use starknet::core::types::StateUpdate;

use std::collections::HashMap;

use httpmock::prelude::*;
use serde_json::json;

use super::super::common::{default_job_item, init_config};
use starknet_core::types::{FieldElement, MaybePendingStateUpdate, StateDiff};
use uuid::Uuid;

use crate::jobs::types::ExternalId;
use crate::jobs::{
    da_job::DaJob,
    types::{JobItem, JobStatus, JobType},
    Job,
};
use da_client_interface::{DaVerificationStatus, MockDaClient};

#[rstest]
#[tokio::test]
async fn test_create_job() {
    let config = init_config(None, None, None, None).await;
    let job = DaJob.create_job(&config, String::from("0"), HashMap::new()).await;
    assert!(job.is_ok());

    let job = job.unwrap();

    let job_type = job.job_type;
    assert_eq!(job_type, JobType::DataSubmission, "job_type should be DataSubmission");
    assert!(!(job.id.is_nil()), "id should not be nil");
    assert_eq!(job.status, JobStatus::Created, "status should be Created");
    assert_eq!(job.version, 0_i32, "version should be 0");
    assert_eq!(job.external_id.unwrap_string().unwrap(), String::new(), "external_id should be empty string");
}

#[rstest]
#[tokio::test]
async fn test_verify_job(#[from(default_job_item)] job_item: JobItem) {
    let mut da_client = MockDaClient::new();
    da_client.expect_verify_inclusion().times(1).returning(|_| Ok(DaVerificationStatus::Verified));

    let config = init_config(None, None, None, Some(da_client)).await;
    assert!(DaJob.verify_job(&config, &job_item).await.is_ok());
}

#[rstest]
#[tokio::test]
async fn test_process_job() {
    let server = MockServer::start();

    let mut da_client = MockDaClient::new();
    let internal_id = "1";
    da_client.expect_publish_state_diff().times(1).returning(|_| Ok(internal_id.to_string()));
    let config = init_config(Some(format!("http://localhost:{}", server.port())), None, None, Some(da_client)).await;

    let state_update = MaybePendingStateUpdate::Update(StateUpdate {
        block_hash: FieldElement::default(),
        new_root: FieldElement::default(),
        old_root: FieldElement::default(),
        state_diff: StateDiff {
            storage_diffs: vec![],
            deprecated_declared_classes: vec![],
            declared_classes: vec![],
            deployed_contracts: vec![],
            replaced_classes: vec![],
            nonces: vec![],
        },
    });
    let state_update = serde_json::to_value(&state_update).unwrap();
    let response = json!({ "id": 1,"jsonrpc":"2.0","result": state_update });

    let state_update_mock = server.mock(|when, then| {
        when.path("/").body_contains("starknet_getStateUpdate");
        then.status(200).body(serde_json::to_vec(&response).unwrap());
    });

    assert_eq!(
        DaJob
            .process_job(
                &config,
                &JobItem {
                    id: Uuid::default(),
                    internal_id: internal_id.to_string(),
                    job_type: JobType::DataSubmission,
                    status: JobStatus::Created,
                    external_id: ExternalId::String("1".to_string().into_boxed_str()),
                    metadata: HashMap::default(),
                    version: 0,
                }
            )
            .await
            .unwrap(),
        internal_id
    );

    state_update_mock.assert();
}
