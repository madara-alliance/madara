use crate::config::config_force_init;
use crate::database::MockDatabase;
use crate::jobs::constants::JOB_METADATA_CAIRO_PIE_PATH_KEY;
use crate::jobs::types::{ExternalId, JobItem, JobStatus, JobType};
use crate::queue::MockQueueProvider;
use crate::tests::common::init_config;
use crate::workers::proving::ProvingWorker;
use crate::workers::Worker;
use da_client_interface::MockDaClient;
use httpmock::MockServer;
use mockall::predicate::eq;
use prover_client_interface::MockProverClient;
use rstest::rstest;
use settlement_client_interface::MockSettlementClient;
use std::collections::HashMap;
use std::error::Error;
use std::time::Duration;
use tokio::time::sleep;
use uuid::Uuid;

#[rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn test_proving_worker(#[case] incomplete_runs: bool) -> Result<(), Box<dyn Error>> {
    let server = MockServer::start();
    let da_client = MockDaClient::new();
    let mut db = MockDatabase::new();
    let mut queue = MockQueueProvider::new();
    let mut prover_client = MockProverClient::new();
    let settlement_client = MockSettlementClient::new();

    sleep(Duration::from_secs(10)).await;

    const JOB_PROCESSING_QUEUE: &str = "madara_orchestrator_job_processing_queue";

    // Mocking Prover Client

    if incomplete_runs {
        let jobs_vec_temp: Vec<JobItem> =
            get_job_item_mock_by_id_vec(5).into_iter().filter(|val| val.internal_id != "3").collect();
        // Mocking db call for getting successful snos jobs
        db.expect_get_jobs_without_successor()
            .times(1)
            .withf(|_, _, _| true)
            .returning(move |_, _, _| Ok(jobs_vec_temp.clone()));

        let num_vec: Vec<i32> = vec![1, 2, 4, 5];

        for i in num_vec {
            db_checks(i, &mut db);
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
            db_checks(i, &mut db);
        }

        // Mocking db call for getting successful snos jobs
        db.expect_get_jobs_without_successor()
            .times(1)
            .withf(|_, _, _| true)
            .returning(move |_, _, _| Ok(get_job_item_mock_by_id_vec(5)));

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
    )
    .await;
    config_force_init(config).await;

    let proving_worker = ProvingWorker {};
    proving_worker.run_worker().await?;

    Ok(())
}

fn get_job_item_mock_by_id_vec(count: i32) -> Vec<JobItem> {
    let mut job_vec: Vec<JobItem> = Vec::new();
    for i in 1..count + 1 {
        let uuid = Uuid::new_v4();
        job_vec.push(JobItem {
            id: uuid,
            internal_id: i.to_string(),
            job_type: JobType::ProofCreation,
            status: JobStatus::Created,
            external_id: ExternalId::Number(0),
            metadata: get_hashmap(),
            version: 0,
        })
    }
    job_vec
}

fn get_job_item_mock_by_id(id: i32) -> JobItem {
    let uuid = Uuid::new_v4();
    JobItem {
        id: uuid,
        internal_id: id.to_string(),
        job_type: JobType::ProofCreation,
        status: JobStatus::Created,
        external_id: ExternalId::Number(0),
        metadata: get_hashmap(),
        version: 0,
    }
}

fn db_checks(id: i32, db: &mut MockDatabase) {
    db.expect_get_job_by_internal_id_and_type()
        .times(1)
        .with(eq(id.clone().to_string()), eq(JobType::ProofCreation))
        .returning(|_, _| Ok(None));

    db.expect_create_job()
        .times(1)
        .withf(move |item| item.internal_id == id.clone().to_string())
        .returning(move |_| Ok(get_job_item_mock_by_id(id)));
}

fn get_hashmap() -> HashMap<String, String> {
    let cairo_pie_path = format!("{}/src/tests/artifacts/fibonacci.zip", env!("CARGO_MANIFEST_DIR"));
    HashMap::from([(JOB_METADATA_CAIRO_PIE_PATH_KEY.into(), cairo_pie_path)])
}
