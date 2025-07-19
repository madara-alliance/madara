use std::error::Error;
use std::sync::Arc;

use crate::core::client::database::MockDatabaseClient;
use crate::core::client::queue::MockQueueClient;
use crate::tests::config::TestConfigBuilder;
use crate::tests::workers::utils::{db_checks_proving_worker, get_job_by_mock_id_vector};
use crate::types::jobs::metadata::JobSpecificMetadata;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::types::queue::QueueType;
use crate::worker::event_handler::factory::mock_factory::get_job_handler_context;
use crate::worker::event_handler::jobs::{JobHandlerTrait, MockJobHandlerTrait};
use crate::worker::event_handler::triggers::proving::ProvingJobTrigger;
use crate::worker::event_handler::triggers::JobTrigger;
use httpmock::MockServer;
use mockall::predicate::eq;
use orchestrator_da_client_interface::MockDaClient;
use orchestrator_prover_client_interface::MockProverClient;
use orchestrator_settlement_client_interface::MockSettlementClient;
use rstest::rstest;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use url::Url;

#[rstest]
#[case(true)]
#[case(false)]
#[tokio::test]
async fn test_proving_worker(#[case] incomplete_runs: bool) -> Result<(), Box<dyn Error>> {
    let num_jobs = 5;
    // Choosing a random incomplete job ID out of the total number of jobs
    let random_incomplete_job_id: u64 = 3;

    let server = MockServer::start();
    let da_client = MockDaClient::new();
    let mut db = MockDatabaseClient::new();
    let mut queue = MockQueueClient::new();
    let prover_client = MockProverClient::new();
    let settlement_client = MockSettlementClient::new();

    // Mocking the get_job_handler function.
    let mut job_handler = MockJobHandlerTrait::new();

    // Create mock SNOS jobs with snos_fact field set
    let mut snos_jobs = Vec::new();

    db.expect_get_orphaned_jobs().returning(|_, _| Ok(Vec::new()));

    for i in 1..=num_jobs {
        // Skip job with ID 3 if incomplete_runs is true
        if incomplete_runs && i == random_incomplete_job_id {
            continue;
        }

        // Create a SNOS job with snos_fact field set
        let mut job = get_job_by_mock_id_vector(JobType::SnosRun, JobStatus::Completed, 1, i)[0].clone();

        // Ensure the SNOS job has a snos_fact field
        if let JobSpecificMetadata::Snos(ref mut snos_metadata) = job.metadata.specific {
            snos_metadata.snos_fact = Some(format!("0x{:064x}", i));
        }

        snos_jobs.push(job);
    }

    // Mock db call for getting successful SNOS jobs without successor
    db.expect_get_jobs_without_successor()
        .times(1)
        .withf(|job_type, job_status, successor_type| {
            *job_type == JobType::SnosRun
                && *job_status == JobStatus::Completed
                && *successor_type == JobType::ProofCreation
        })
        .returning(move |_, _, _| Ok(snos_jobs.clone()));

    // Set up expectations for each job
    for i in 1..=num_jobs {
        if incomplete_runs && i == random_incomplete_job_id {
            continue;
        }
        db_checks_proving_worker(i as i32, &mut db, &mut job_handler);
    }

    // Queue function call simulations
    if incomplete_runs {
        queue
            .expect_send_message()
            .times(4)
            .returning(|_, _, _| Ok(()))
            .withf(|queue, _payload, _delay| *queue == QueueType::ProvingJobProcessing);
    } else {
        queue
            .expect_send_message()
            .times(5)
            .returning(|_, _, _| Ok(()))
            .withf(|queue, _payload, _delay| *queue == QueueType::ProvingJobProcessing);
    }
    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(format!("http://localhost:{}", server.port()).as_str()).expect("Failed to parse URL"),
    ));

    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_database(db.into())
        .configure_queue_client(queue.into())
        .configure_da_client(da_client.into())
        .configure_prover_client(prover_client.into())
        .configure_settlement_client(settlement_client.into())
        .build()
        .await;

    let job_handler: Arc<Box<dyn JobHandlerTrait>> = Arc::new(Box::new(job_handler));
    let ctx = get_job_handler_context();

    // Mocking the `get_job_handler` call in create_job function.
    if incomplete_runs {
        ctx.expect().times(4).with(eq(JobType::ProofCreation)).returning(move |_| Arc::clone(&job_handler));
    } else {
        ctx.expect().times(5).with(eq(JobType::ProofCreation)).returning(move |_| Arc::clone(&job_handler));
    }

    ProvingJobTrigger.run_worker(services.config).await?;

    Ok(())
}
