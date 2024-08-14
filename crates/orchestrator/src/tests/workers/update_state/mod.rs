use std::error::Error;
use std::sync::Arc;

use da_client_interface::MockDaClient;
use httpmock::MockServer;
use mockall::predicate::eq;
use rstest::rstest;
use uuid::Uuid;

use crate::config::config_force_init;
use crate::database::MockDatabase;
use crate::jobs::job_handler_factory::mock_factory;
use crate::jobs::types::{JobStatus, JobType};
use crate::jobs::{Job, MockJob};
use crate::queue::MockQueueProvider;
use crate::tests::common::init_config;
use crate::tests::workers::utils::{get_job_by_mock_id_vector, get_job_item_mock_by_id};
use crate::workers::update_state::UpdateStateWorker;
use crate::workers::Worker;

#[rstest]
#[case(false, 0)]
#[case(true, 5)]
#[tokio::test]
async fn test_update_state_worker(
    #[case] last_successful_job_exists: bool,
    #[case] number_of_processed_jobs: usize,
) -> Result<(), Box<dyn Error>> {
    let server = MockServer::start();
    let da_client = MockDaClient::new();
    let mut db = MockDatabase::new();
    let mut queue = MockQueueProvider::new();

    const JOB_PROCESSING_QUEUE: &str = "madara_orchestrator_job_processing_queue";

    // Mocking the get_job_handler function.
    let mut job_handler = MockJob::new();

    // Mocking db function expectations
    // If no successful state update jobs exist
    if !last_successful_job_exists {
        db.expect_get_latest_job_by_type_and_status()
            .with(eq(JobType::StateTransition), eq(JobStatus::Completed))
            .times(1)
            .returning(|_, _| Ok(None));
    } else {
        // if successful state update job exists

        // mocking the return value of first function call (getting last successful jobs):
        db.expect_get_latest_job_by_type_and_status()
            .with(eq(JobType::StateTransition), eq(JobStatus::Completed))
            .times(1)
            .returning(|_, _| Ok(Some(get_job_item_mock_by_id("1".to_string(), Uuid::new_v4()))));

        // mocking the return values of second function call (getting completed proving worker jobs)
        db.expect_get_jobs_after_internal_id_by_job_type()
            .with(eq(JobType::ProofCreation), eq(JobStatus::Completed), eq("1".to_string()))
            .returning(move |_, _, _| {
                Ok(get_job_by_mock_id_vector(
                    JobType::ProofCreation,
                    JobStatus::Completed,
                    number_of_processed_jobs as u64,
                    2,
                ))
            });

        // mocking getting of the jobs (when there is a safety check for any pre-existing job during job
        // creation)
        let completed_jobs =
            get_job_by_mock_id_vector(JobType::ProofCreation, JobStatus::Completed, number_of_processed_jobs as u64, 2);
        for job in completed_jobs {
            db.expect_get_job_by_internal_id_and_type()
                .times(1)
                .with(eq(job.internal_id.to_string()), eq(JobType::StateTransition))
                .returning(|_, _| Ok(None));
        }

        // mocking the creation of jobs
        let job_item = get_job_item_mock_by_id("1".to_string(), Uuid::new_v4());
        let job_item_cloned = job_item.clone();

        job_handler.expect_create_job().times(1).returning(move |_, _, _| Ok(job_item.clone()));

        db.expect_create_job()
            .times(1)
            .withf(move |item| item.internal_id == *"1".to_string())
            .returning(move |_| Ok(job_item_cloned.clone()));
    }

    let y: Arc<Box<dyn Job>> = Arc::new(Box::new(job_handler));
    let ctx = mock_factory::get_job_handler_context();
    // Mocking the `get_job_handler` call in create_job function.
    if last_successful_job_exists {
        ctx.expect().times(1).with(eq(JobType::StateTransition)).returning(move |_| Arc::clone(&y));
    }

    // Queue function call simulations
    queue
        .expect_send_message_to_queue()
        .returning(|_, _, _| Ok(()))
        .withf(|queue, _payload, _delay| queue == JOB_PROCESSING_QUEUE);

    // mock block number (madara) : 5
    let config = init_config(
        Some(format!("http://localhost:{}", server.port())),
        Some(db),
        Some(queue),
        Some(da_client),
        None,
        None,
        None,
    )
    .await;
    config_force_init(config).await;

    let update_state_worker = UpdateStateWorker {};
    update_state_worker.run_worker().await?;

    Ok(())
}
