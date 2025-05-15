use crate::core::client::database::DatabaseError;
use crate::tests::config::{ConfigType, TestConfigBuilder};
use crate::tests::utils::{build_batch, build_job_item};
use crate::types::batch::BatchUpdates;
use crate::types::jobs::job_updates::JobItemUpdates;
use crate::types::jobs::metadata::JobSpecificMetadata;
use crate::types::jobs::types::{JobStatus, JobType};
use rstest::*;

#[rstest]
#[tokio::test]
async fn test_database_connection() -> color_eyre::Result<()> {
    let _services = TestConfigBuilder::new().build().await;
    Ok(())
}

/// Tests for `create_job` operation in database trait.
/// Creates 3 jobs and asserts them.
#[rstest]
#[tokio::test]
async fn database_create_job_works() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    let job_vec = [
        build_job_item(JobType::ProofCreation, JobStatus::Created, 1),
        build_job_item(JobType::ProofCreation, JobStatus::Created, 2),
        build_job_item(JobType::ProofCreation, JobStatus::Created, 3),
    ];

    database_client.create_job(job_vec[0].clone()).await.unwrap();
    database_client.create_job(job_vec[1].clone()).await.unwrap();
    database_client.create_job(job_vec[2].clone()).await.unwrap();

    let get_job_1 =
        database_client.get_job_by_internal_id_and_type("1", &JobType::ProofCreation).await.unwrap().unwrap();
    let get_job_2 =
        database_client.get_job_by_internal_id_and_type("2", &JobType::ProofCreation).await.unwrap().unwrap();
    let get_job_3 =
        database_client.get_job_by_internal_id_and_type("3", &JobType::ProofCreation).await.unwrap().unwrap();

    assert_eq!(get_job_1, job_vec[0].clone());
    assert_eq!(get_job_2, job_vec[1].clone());
    assert_eq!(get_job_3, job_vec[2].clone());
}

/// Tests for `create_job` operation in database trait.
/// Creates a job with the same job type and internal id as an existing job.
/// Should fail.
#[rstest]
#[tokio::test]
async fn database_create_job_with_job_exists_fails() {
    let services: crate::tests::config::TestConfigBuilderReturns =
        TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    let job_one = build_job_item(JobType::ProofCreation, JobStatus::Created, 1);

    // same job type and internal id
    let job_two = build_job_item(JobType::ProofCreation, JobStatus::LockedForProcessing, 1);

    database_client.create_job(job_one).await.unwrap();

    let result = database_client.create_job(job_two).await;

    // let result_err = result.unwrap_err();

    assert!(matches!(result, Err(DatabaseError::ItemAlreadyExists(_))));
    // fetch job to see the status wasn't updated
    let fetched_job =
        database_client.get_job_by_internal_id_and_type("1", &JobType::ProofCreation).await.unwrap().unwrap();
    assert_eq!(fetched_job.status, JobStatus::Created);
}

/// Test for `get_jobs_without_successor` operation in database trait.
/// Creates jobs in the following sequence :
///
/// - Creates 3 snos run jobs with completed status
///
/// - Creates 2 proof creation jobs with succession of the 2 snos jobs
///
/// - Should return one snos job without the successor job of proof creation
#[rstest]
#[case(true)]
#[case(false)]
#[tokio::test]
async fn database_get_jobs_without_successor_works(#[case] is_successor: bool) {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    let job_vec = [
        build_job_item(JobType::SnosRun, JobStatus::Completed, 1),
        build_job_item(JobType::SnosRun, JobStatus::Completed, 2),
        build_job_item(JobType::SnosRun, JobStatus::Completed, 3),
        build_job_item(JobType::ProofCreation, JobStatus::Created, 1),
        build_job_item(JobType::ProofCreation, JobStatus::Created, 2),
        build_job_item(JobType::ProofCreation, JobStatus::Created, 3),
    ];

    database_client.create_job(job_vec[0].clone()).await.unwrap();
    database_client.create_job(job_vec[1].clone()).await.unwrap();
    database_client.create_job(job_vec[2].clone()).await.unwrap();
    database_client.create_job(job_vec[3].clone()).await.unwrap();
    database_client.create_job(job_vec[5].clone()).await.unwrap();
    if is_successor {
        database_client.create_job(job_vec[4].clone()).await.unwrap();
    }

    let jobs_without_successor = database_client
        .get_jobs_without_successor(JobType::SnosRun, JobStatus::Completed, JobType::ProofCreation)
        .await
        .unwrap();

    if is_successor {
        assert_eq!(jobs_without_successor.len(), 0, "Expected number of jobs assertion failed.");
    } else {
        assert_eq!(jobs_without_successor.len(), 1, "Expected number of jobs assertion failed.");
        assert_eq!(jobs_without_successor[0], job_vec[1], "Expected job assertion failed.");
    }
}

/// Test for `get_latest_job_by_type` operation in database trait.
/// Creates the jobs in following sequence :
///
/// - Creates 3 successful jobs.
///
/// - Should return the last successful job
#[rstest]
#[tokio::test]
async fn database_get_last_successful_job_by_type_works() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    let job_vec = [
        build_job_item(JobType::SnosRun, JobStatus::Completed, 1),
        build_job_item(JobType::SnosRun, JobStatus::Completed, 2),
        build_job_item(JobType::SnosRun, JobStatus::Completed, 3),
    ];

    database_client.create_job(job_vec[0].clone()).await.unwrap();
    database_client.create_job(job_vec[1].clone()).await.unwrap();
    database_client.create_job(job_vec[2].clone()).await.unwrap();

    let last_successful_job = database_client.get_latest_job_by_type(JobType::SnosRun).await.unwrap().unwrap();

    assert_eq!(last_successful_job, job_vec[2], "Expected job assertion failed");
}

/// Test for `get_jobs_after_internal_id_by_job_type` operation in database trait.
/// Creates the jobs in following sequence :
///
/// - Creates 5 successful jobs.
///
/// - Should return the jobs after internal id
#[rstest]
#[tokio::test]
async fn database_get_jobs_after_internal_id_by_job_type_works() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    let job_vec = [
        build_job_item(JobType::SnosRun, JobStatus::Completed, 1),
        build_job_item(JobType::SnosRun, JobStatus::Completed, 2),
        build_job_item(JobType::ProofCreation, JobStatus::Completed, 3),
        build_job_item(JobType::ProofCreation, JobStatus::Completed, 4),
        build_job_item(JobType::SnosRun, JobStatus::Completed, 5),
        build_job_item(JobType::SnosRun, JobStatus::Completed, 6),
    ];

    database_client.create_job(job_vec[0].clone()).await.unwrap();
    database_client.create_job(job_vec[1].clone()).await.unwrap();
    database_client.create_job(job_vec[2].clone()).await.unwrap();
    database_client.create_job(job_vec[3].clone()).await.unwrap();
    database_client.create_job(job_vec[4].clone()).await.unwrap();
    database_client.create_job(job_vec[5].clone()).await.unwrap();

    let jobs_after_internal_id = database_client
        .get_jobs_after_internal_id_by_job_type(JobType::SnosRun, JobStatus::Completed, "2".to_string())
        .await
        .unwrap();

    assert_eq!(jobs_after_internal_id.len(), 2, "Number of jobs assertion failed");
    assert_eq!(jobs_after_internal_id[0], job_vec[4]);
    assert_eq!(jobs_after_internal_id[1], job_vec[5]);
}

#[rstest]
#[tokio::test]
async fn database_test_update_job() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    let job = build_job_item(JobType::DataSubmission, JobStatus::Created, 456);
    database_client.create_job(job.clone()).await.unwrap();

    let job_id = job.id;

    // Create updated metadata with the new structure
    let mut updated_job_metadata = job.metadata.clone();
    if let JobSpecificMetadata::Da(ref mut da_metadata) = updated_job_metadata.specific {
        da_metadata.block_number = 456;
        da_metadata.tx_hash = Some("test_key".to_string());
    }

    let job_cloned = job.clone();
    let updated_job = database_client
        .update_job(
            &job_cloned,
            JobItemUpdates::new()
                .update_status(JobStatus::LockedForProcessing)
                .update_metadata(updated_job_metadata)
                .build(),
        )
        .await;

    if let Some(job_after_updates_db) = database_client.get_job_by_id(job_id).await.unwrap() {
        // check if job is updated
        assert_eq!(JobType::DataSubmission, job_after_updates_db.job_type);
        assert_eq!(JobStatus::LockedForProcessing, job_after_updates_db.status);
        assert_eq!(1, job_after_updates_db.version);
        assert_eq!(456.to_string(), job_after_updates_db.internal_id);

        // Check metadata was updated correctly
        if let JobSpecificMetadata::Da(da_metadata) = &job_after_updates_db.metadata.specific {
            assert_eq!(Some("test_key".to_string()), da_metadata.tx_hash);
        } else {
            panic!("Wrong metadata type");
        }

        // check if value returned by `update_job` is the correct one
        // and matches the one in database
        assert_eq!(updated_job.unwrap(), job_after_updates_db);
    } else {
        panic!("Job not found in Database.")
    }
}

#[rstest]
#[tokio::test]
async fn database_test_get_latest_batch() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    // Create multiple batches with different indices
    let batch1 = build_batch(1, 100, 200);
    let batch2 = build_batch(2, 201, 300);
    let batch3 = build_batch(3, 301, 400);

    // Insert batches in non-sequential order
    database_client.create_batch(batch2.clone()).await.unwrap();
    database_client.create_batch(batch1.clone()).await.unwrap();
    database_client.create_batch(batch3.clone()).await.unwrap();

    // Get latest batch should return batch3 since it has the highest index
    let latest_batch = database_client.get_latest_batch().await.unwrap().unwrap();
    assert_eq!(latest_batch, batch3);
}

#[rstest]
#[tokio::test]
async fn database_test_update_batch() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    // Create a new batch
    let batch = build_batch(1, 100, 200);

    database_client.create_batch(batch.clone()).await.unwrap();

    // Waiting for sometime to ensure updated_at is different after the update
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Create updates for the batch
    let updates = BatchUpdates { end_block: 250, is_batch_ready: batch.is_batch_ready };

    // Update the batch
    let updated_batch = database_client.update_batch(&batch, &updates).await.unwrap();

    // Verify the updates
    assert_eq!(updated_batch.id, batch.id);
    assert_eq!(updated_batch.index, batch.index);
    assert_eq!(updated_batch.size, updates.end_block - batch.start_block + 1);
    assert_eq!(updated_batch.start_block, batch.start_block);
    assert_eq!(updated_batch.end_block, updates.end_block);
    assert_eq!(updated_batch.is_batch_ready, batch.is_batch_ready);
    assert_eq!(updated_batch.squashed_state_updates_path, batch.squashed_state_updates_path);
    assert_eq!(updated_batch.created_at, batch.created_at);
    assert_ne!(updated_batch.updated_at, batch.updated_at);
}

#[rstest]
#[tokio::test]
async fn database_test_create_batch() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    // Create a new batch
    let batch = build_batch(1, 100, 200);

    // Create the batch
    let created_batch = database_client.create_batch(batch.clone()).await.unwrap();

    // Verify the created batch matches the input
    assert_eq!(created_batch, batch);

    // Verify we can retrieve the batch
    let retrieved_batch = database_client.get_latest_batch().await.unwrap().unwrap();
    assert_eq!(retrieved_batch, batch);
}
