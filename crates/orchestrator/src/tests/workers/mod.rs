use crate::config::config_force_init;
use crate::database::MockDatabase;
use crate::jobs::types::{ExternalId, JobItem, JobStatus, JobType};
use crate::queue::MockQueueProvider;
use crate::tests::common::init_config;
use crate::workers::snos::SnosWorker;
use crate::workers::Worker;
use da_client_interface::MockDaClient;
use httpmock::MockServer;
use mockall::predicate::eq;
use rstest::rstest;
use serde_json::json;
use std::collections::HashMap;
use std::error::Error;
use uuid::Uuid;

#[rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn test_snos_worker(#[case] db_val: bool) -> Result<(), Box<dyn Error>> {
    let server = MockServer::start();
    let da_client = MockDaClient::new();
    let mut db = MockDatabase::new();
    let mut queue = MockQueueProvider::new();
    let start_job_index;
    let block;

    const JOB_PROCESSING_QUEUE: &str = "madara_orchestrator_job_processing_queue";

    // Mocking db function expectations
    if !db_val {
        db.expect_get_latest_job_by_type_and_internal_id().times(1).with(eq(JobType::SnosRun)).returning(|_| Ok(None));
        start_job_index = 1;
        block = 5;
    } else {
        let uuid_temp = Uuid::new_v4();

        db.expect_get_latest_job_by_type_and_internal_id()
            .with(eq(JobType::SnosRun))
            .returning(move |_| Ok(Some(get_job_item_mock_by_id("1".to_string(), uuid_temp))));
        block = 6;
        start_job_index = 2;
    }

    for i in start_job_index..block + 1 {
        // Getting jobs for check expectations
        db.expect_get_job_by_internal_id_and_type()
            .times(1)
            .with(eq(i.clone().to_string()), eq(JobType::SnosRun))
            .returning(|_, _| Ok(None));

        let uuid = Uuid::new_v4();

        // creating jobs call expectations
        db.expect_create_job()
            .times(1)
            .withf(move |item| item.internal_id == i.clone().to_string())
            .returning(move |_| Ok(get_job_item_mock_by_id(i.clone().to_string(), uuid)));
    }

    // Queue function call simulations
    queue
        .expect_send_message_to_queue()
        .returning(|_, _, _| Ok(()))
        .withf(|queue, _payload, _delay| queue == JOB_PROCESSING_QUEUE);

    // mock block number (madara) : 5
    let rpc_response_block_number = block;
    let response = json!({ "id": 1,"jsonrpc":"2.0","result": rpc_response_block_number });
    let config =
        init_config(Some(format!("http://localhost:{}", server.port())), Some(db), Some(queue), Some(da_client)).await;
    config_force_init(config).await;

    // mocking block call
    let rpc_block_call_mock = server.mock(|when, then| {
        when.path("/").body_contains("starknet_blockNumber");
        then.status(200).body(serde_json::to_vec(&response).unwrap());
    });

    let snos_worker = SnosWorker {};
    snos_worker.run_worker().await?;

    rpc_block_call_mock.assert();

    Ok(())
}

fn get_job_item_mock_by_id(id: String, uuid: Uuid) -> JobItem {
    JobItem {
        id: uuid,
        internal_id: id.clone(),
        job_type: JobType::SnosRun,
        status: JobStatus::Created,
        external_id: ExternalId::Number(0),
        metadata: HashMap::new(),
        version: 0,
    }
}
