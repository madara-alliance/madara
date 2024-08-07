use std::error::Error;
use std::sync::Arc;

use da_client_interface::MockDaClient;
use httpmock::MockServer;
use mockall::predicate::eq;
use prover_client_interface::MockProverClient;
use rstest::rstest;
use settlement_client_interface::MockSettlementClient;

use crate::config::config_force_init;
use crate::database::MockDatabase;
use crate::jobs::job_handler_factory::mock_factory;
use crate::jobs::types::{JobItem, JobStatus, JobType};
use crate::jobs::{Job, MockJob};
use crate::queue::MockQueueProvider;
use crate::tests::common::init_config;
use crate::tests::workers::utils::{db_checks_proving_worker, get_job_by_mock_id_vector};
use crate::workers::proving::ProvingWorker;

#[rstest]
#[case(true)]
#[case(false)]
#[tokio::test]
async fn test_proving_worker(#[case] incomplete_runs: bool) -> Result<(), Box<dyn Error>> {
    let server = MockServer::start();
    let da_client = MockDaClient::new();
    let mut db = MockDatabase::new();
    let mut queue = MockQueueProvider::new();
    let mut prover_client = MockProverClient::new();
    let settlement_client = MockSettlementClient::new();

    const JOB_PROCESSING_QUEUE: &str = "madara_orchestrator_job_processing_queue";

    // Mocking Prover Client

    // Mocking the get_job_handler function.
    let mut job_handler = MockJob::new();

    // incomplete_runs : This refers to if there are incomplete runs in the previous job which is
    // `snos_job` in this case.
    if incomplete_runs {
        let jobs_vec_temp: Vec<JobItem> = get_job_by_mock_id_vector(JobType::ProofCreation, JobStatus::Created, 5, 1)
            .into_iter()
            .filter(|val| val.internal_id != "3")
            .collect();
        // Mocking db call for getting successful snos jobs
        db.expect_get_jobs_without_successor()
            .times(1)
            .withf(|_, _, _| true)
            .returning(move |_, _, _| Ok(jobs_vec_temp.clone()));

        let num_vec: Vec<i32> = vec![1, 2, 4, 5];

        for i in num_vec {
            db_checks_proving_worker(i, &mut db, &mut job_handler);
        }

        prover_client.expect_submit_task().times(4).returning(|_| Ok("task_id".to_string()));

        // Queue function call simulations
        queue
            .expect_send_message_to_queue()
            .times(4)
            .returning(|_, _, _| Ok(()))
            .withf(|queue, _payload, _delay| queue == JOB_PROCESSING_QUEUE);
    } else {
        for i in 1..5 + 1 {
            db_checks_proving_worker(i, &mut db, &mut job_handler);
        }

        // Mocking db call for getting successful snos jobs
        db.expect_get_jobs_without_successor()
            .times(1)
            .withf(|_, _, _| true)
            .returning(move |_, _, _| Ok(get_job_by_mock_id_vector(JobType::ProofCreation, JobStatus::Created, 5, 1)));

        prover_client.expect_submit_task().times(5).returning(|_| Ok("task_id".to_string()));

        // Queue function call simulations
        queue
            .expect_send_message_to_queue()
            .times(5)
            .returning(|_, _, _| Ok(()))
            .withf(|queue, _payload, _delay| queue == JOB_PROCESSING_QUEUE);
    }

    let config = init_config(
        Some(format!("http://localhost:{}", server.port())),
        Some(db),
        Some(queue),
        Some(da_client),
        Some(prover_client),
        Some(settlement_client),
        None,
    )
    .await;
    config_force_init(config).await;

    let job_handler: Arc<Box<dyn Job>> = Arc::new(Box::new(job_handler));
    let ctx = mock_factory::get_job_handler_context();
    // Mocking the `get_job_handler` call in create_job function.
    if incomplete_runs {
        ctx.expect().times(4).with(eq(JobType::ProofCreation)).returning(move |_| Arc::clone(&job_handler));
    } else {
        ctx.expect().times(5).with(eq(JobType::ProofCreation)).returning(move |_| Arc::clone(&job_handler));
    }

    let proving_worker = ProvingWorker {};
    proving_worker.run_worker().await?;

    Ok(())
}

use crate::workers::Worker;
