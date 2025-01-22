use std::error::Error;
use std::sync::Arc;

use da_client_interface::MockDaClient;
use httpmock::MockServer;
use mockall::predicate::eq;
use rstest::rstest;
use serde_json::json;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use url::Url;
use uuid::Uuid;

use crate::database::MockDatabase;
use crate::jobs::job_handler_factory::mock_factory;
use crate::jobs::types::JobType;
use crate::jobs::{Job, MockJob};
use crate::queue::MockQueueProvider;
use crate::tests::config::TestConfigBuilder;
use crate::tests::workers::utils::get_job_item_mock_by_id;
use crate::workers::snos::SnosWorker;
use crate::workers::Worker;

#[rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn test_snos_worker(#[case] db_val: bool) -> Result<(), Box<dyn Error>> {
    use crate::queue::QueueType;

    let server = MockServer::start();
    let da_client = MockDaClient::new();
    let mut db = MockDatabase::new();
    let mut queue = MockQueueProvider::new();
    let start_job_index;
    let block;

    // Mocking the get_job_handler function.
    let mut job_handler = MockJob::new();

    // Mocking db function expectations
    if !db_val {
        db.expect_get_latest_job_by_type().with(eq(JobType::SnosRun)).returning(|_| Ok(None));
        db.expect_get_job_by_internal_id_and_type()
            .with(eq(0.to_string()), eq(JobType::SnosRun))
            .returning(|_, _| Ok(None));
        db.expect_get_job_by_internal_id_and_type()
            .with(eq(1.to_string()), eq(JobType::SnosRun))
            .returning(|_, _| Ok(None));

        start_job_index = 0;
        block = 5;
    } else {
        let uuid_temp = Uuid::new_v4();
        db.expect_get_latest_job_by_type()
            .with(eq(JobType::SnosRun))
            .returning(move |_| Ok(Some(get_job_item_mock_by_id("1".to_string(), uuid_temp))));
        db.expect_get_job_by_internal_id_and_type()
            .with(eq(0.to_string()), eq(JobType::SnosRun))
            .returning(move |_, _| Ok(Some(get_job_item_mock_by_id("0".to_string(), uuid_temp))));
        db.expect_get_job_by_internal_id_and_type()
            .with(eq(1.to_string()), eq(JobType::SnosRun))
            .returning(move |_, _| Ok(Some(get_job_item_mock_by_id("1".to_string(), uuid_temp))));

        start_job_index = 2;
        block = 6;
    }

    for i in start_job_index..block + 1 {
        // Getting jobs for check expectations
        db.expect_get_job_by_internal_id_and_type()
            .with(eq(i.clone().to_string()), eq(JobType::SnosRun))
            .returning(|_, _| Ok(None));

        let uuid = Uuid::new_v4();

        let job_item = get_job_item_mock_by_id(i.clone().to_string(), uuid);
        let job_item_cloned = job_item.clone();
        job_handler.expect_create_job().returning(move |_, _, _| Ok(job_item_cloned.clone()));

        // creating jobs call expectations
        db.expect_create_job()
            .withf(move |item| item.internal_id == i.clone().to_string())
            .returning(move |_| Ok(job_item.clone()));
    }

    let job_handler: Arc<Box<dyn Job>> = Arc::new(Box::new(job_handler));
    let ctx = mock_factory::get_job_handler_context();
    // Mocking the `get_job_handler` call in create_job function.
    ctx.expect().with(eq(JobType::SnosRun)).returning(move |_| Arc::clone(&job_handler));

    // Queue function call simulations
    queue
        .expect_send_message_to_queue()
        .returning(|_, _, _| Ok(()))
        .withf(|queue, _payload, _delay| *queue == QueueType::SnosJobProcessing);

    // mock block number (madara) : 5
    let rpc_response_block_number = block;
    let response = json!({ "id": 1,"jsonrpc":"2.0","result": rpc_response_block_number });

    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(format!("http://localhost:{}", server.port()).as_str()).expect("Failed to parse URL"),
    ));

    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_database(db.into())
        .configure_queue_client(queue.into())
        .configure_da_client(da_client.into())
        .build()
        .await;

    // mocking block call
    let rpc_block_call_mock = server.mock(|when, then| {
        when.path("/").body_includes("starknet_blockNumber");
        then.status(200).body(serde_json::to_vec(&response).unwrap());
    });

    let snos_worker = SnosWorker {};
    snos_worker.run_worker(services.config).await?;

    rpc_block_call_mock.assert();

    Ok(())
}
