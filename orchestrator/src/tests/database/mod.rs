use crate::core::client::database::DatabaseError;
use crate::tests::config::{ConfigType, TestConfigBuilder};
use crate::tests::utils::{
    build_batch, build_batch_with_version, build_job_item, build_job_item_with_version, build_snos_batch,
    build_snos_batch_with_version,
};
use crate::types::batch::{AggregatorBatch, AggregatorBatchStatus, AggregatorBatchUpdates, SnosBatchStatus};
use crate::types::jobs::job_updates::JobItemUpdates;
use crate::types::jobs::metadata::JobSpecificMetadata;
use crate::types::jobs::types::{JobStatus, JobType};
use chrono::Utc;
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

    let get_job_1 = database_client.get_job_by_internal_id_and_type(1, &JobType::ProofCreation).await.unwrap().unwrap();
    let get_job_2 = database_client.get_job_by_internal_id_and_type(2, &JobType::ProofCreation).await.unwrap().unwrap();
    let get_job_3 = database_client.get_job_by_internal_id_and_type(3, &JobType::ProofCreation).await.unwrap().unwrap();

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
        database_client.get_job_by_internal_id_and_type(1, &JobType::ProofCreation).await.unwrap().unwrap();
    assert_eq!(fetched_job.status, JobStatus::Created);
}

/// Test for `get_jobs_without_successor` operation in database trait.
/// Creates jobs in the following sequence :
///
/// - Creates 3 snos run jobs with completed status (2 with current version, 1 with old version)
///
/// - Creates 2 proof creation jobs with succession of the 2 snos jobs
///
/// - Tests both successor logic and orchestrator_version filtering
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
        build_job_item_with_version(JobType::SnosRun, JobStatus::Completed, 4, "old-version".to_string()),
    ];

    database_client.create_job(job_vec[0].clone()).await.unwrap();
    database_client.create_job(job_vec[1].clone()).await.unwrap();
    database_client.create_job(job_vec[2].clone()).await.unwrap();
    database_client.create_job(job_vec[3].clone()).await.unwrap();
    database_client.create_job(job_vec[5].clone()).await.unwrap();
    database_client.create_job(job_vec[6].clone()).await.unwrap();
    if is_successor {
        database_client.create_job(job_vec[4].clone()).await.unwrap();
    }

    // Test without version filter
    let jobs_without_successor = database_client
        .get_jobs_without_successor(JobType::SnosRun, JobStatus::Completed, JobType::ProofCreation, None)
        .await
        .unwrap();

    if is_successor {
        // job_vec[6] (old version) has no successor
        assert_eq!(jobs_without_successor.len(), 1, "Expected 1 job without successor (old version job)");
        assert_eq!(jobs_without_successor[0].internal_id, 4);
    } else {
        // job_vec[1] and job_vec[2] have no successors
        assert_eq!(jobs_without_successor.len(), 2, "Expected 2 jobs without successor"); // snos jobs 2 and 4
    }

    // Test with current version filter
    let current_version = crate::types::constant::ORCHESTRATOR_VERSION.to_string();
    let jobs_current_version = database_client
        .get_jobs_without_successor(
            JobType::SnosRun,
            JobStatus::Completed,
            JobType::ProofCreation,
            Some(current_version),
        )
        .await
        .unwrap();

    if is_successor {
        assert_eq!(jobs_current_version.len(), 0, "All current version jobs have successors");
    } else {
        assert_eq!(jobs_current_version.len(), 1, "Only job 2 with current version has no successor");
        assert_eq!(jobs_current_version[0].internal_id, 2);
    }

    // Test with old version filter
    let jobs_old_version = database_client
        .get_jobs_without_successor(
            JobType::SnosRun,
            JobStatus::Completed,
            JobType::ProofCreation,
            Some("old-version".to_string()),
        )
        .await
        .unwrap();

    // Job 3 (old version) never has a successor in this test
    assert_eq!(jobs_old_version.len(), 1, "Old version job has no successor");
    assert_eq!(jobs_old_version[0].internal_id, 4);
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

/// Test for `get_oldest_job_by_type_excluding_statuses` operation in database trait.
/// Creates jobs with different statuses and verifies the method returns the oldest job
/// whose status is not in the excluded list.
#[rstest]
#[tokio::test]
async fn database_get_oldest_job_by_type_excluding_statuses_works() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    // Create jobs with different statuses (internal_id determines order)
    // internal_id=1: Completed (should be excluded)
    // internal_id=2: Completed (should be excluded)
    // internal_id=3: Created (oldest non-completed, should be returned)
    // internal_id=4: LockedForProcessing
    // internal_id=5: PendingVerification
    let job_vec = [
        build_job_item(JobType::SnosRun, JobStatus::Completed, 1),
        build_job_item(JobType::SnosRun, JobStatus::Completed, 2),
        build_job_item(JobType::SnosRun, JobStatus::Created, 3),
        build_job_item(JobType::SnosRun, JobStatus::LockedForProcessing, 4),
        build_job_item(JobType::SnosRun, JobStatus::PendingVerification, 5),
    ];

    for job in &job_vec {
        database_client.create_job(job.clone()).await.unwrap();
    }

    // Exclude Completed status - should return job with internal_id=3 (oldest non-completed)
    let oldest_incomplete = database_client
        .get_oldest_job_by_type_excluding_statuses(JobType::SnosRun, vec![JobStatus::Completed])
        .await
        .unwrap();

    assert!(oldest_incomplete.is_some(), "Expected to find an incomplete job");
    let oldest_job = oldest_incomplete.unwrap();
    assert_eq!(oldest_job.internal_id, 3, "Expected oldest non-completed job to have internal_id=3");
    assert_eq!(oldest_job.status, JobStatus::Created);

    // Exclude both Completed and Created - should return job with internal_id=4
    let oldest_excluding_multiple = database_client
        .get_oldest_job_by_type_excluding_statuses(JobType::SnosRun, vec![JobStatus::Completed, JobStatus::Created])
        .await
        .unwrap();

    assert!(oldest_excluding_multiple.is_some());
    let job = oldest_excluding_multiple.unwrap();
    assert_eq!(job.internal_id, 4, "Expected oldest job excluding Completed and Created to have internal_id=4");
    assert_eq!(job.status, JobStatus::LockedForProcessing);
}

/// Test for `get_jobs_after_internal_id_by_job_type` operation in database trait.
/// Creates the jobs in following sequence :
///
/// - Creates 6 jobs (4 SnosRun with mixed versions, 2 ProofCreation)
///
/// - Tests both internal_id filtering and orchestrator_version filtering
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
        build_job_item_with_version(JobType::SnosRun, JobStatus::Completed, 6, "old-version".to_string()),
    ];

    database_client.create_job(job_vec[0].clone()).await.unwrap();
    database_client.create_job(job_vec[1].clone()).await.unwrap();
    database_client.create_job(job_vec[2].clone()).await.unwrap();
    database_client.create_job(job_vec[3].clone()).await.unwrap();
    database_client.create_job(job_vec[4].clone()).await.unwrap();
    database_client.create_job(job_vec[5].clone()).await.unwrap();

    // Test without version filter - should return all SnosRun jobs after id 2
    let jobs_after_internal_id = database_client
        .get_jobs_after_internal_id_by_job_type(JobType::SnosRun, JobStatus::Completed, 2, None)
        .await
        .unwrap();

    assert_eq!(jobs_after_internal_id.len(), 2, "Should return 2 SnosRun jobs after id 2");
    assert_eq!(jobs_after_internal_id[0].internal_id, 5);
    assert_eq!(jobs_after_internal_id[1].internal_id, 6);

    // Test with current version filter - should only return job 5
    let current_version = crate::types::constant::ORCHESTRATOR_VERSION.to_string();
    let jobs_current_version = database_client
        .get_jobs_after_internal_id_by_job_type(JobType::SnosRun, JobStatus::Completed, 2, Some(current_version))
        .await
        .unwrap();

    assert_eq!(jobs_current_version.len(), 1, "Should return 1 job with current version after id 2");
    assert_eq!(jobs_current_version[0].internal_id, 5);

    // Test with old version filter - should only return job 6
    let jobs_old_version = database_client
        .get_jobs_after_internal_id_by_job_type(
            JobType::SnosRun,
            JobStatus::Completed,
            2,
            Some("old-version".to_string()),
        )
        .await
        .unwrap();

    assert_eq!(jobs_old_version.len(), 1, "Should return 1 job with old version after id 2");
    assert_eq!(jobs_old_version[0].internal_id, 6);
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
        assert_eq!(456, job_after_updates_db.internal_id);

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
async fn database_test_get_latest_batch(
    #[from(build_batch)]
    #[with(1, 100, 200)]
    batch1: AggregatorBatch,
    #[from(build_batch)]
    #[with(2, 210, 300)]
    batch2: AggregatorBatch,
    #[from(build_batch)]
    #[with(3, 301, 400)]
    batch3: AggregatorBatch,
) {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    // Insert batches in non-sequential order
    database_client.create_aggregator_batch(batch2.clone()).await.unwrap();
    database_client.create_aggregator_batch(batch1.clone()).await.unwrap();
    database_client.create_aggregator_batch(batch3.clone()).await.unwrap();

    // Get latest batch should return batch3 since it has the highest index
    let latest_batch = database_client.get_latest_aggregator_batch().await.unwrap().unwrap();
    assert_eq!(latest_batch, batch3);
}

#[rstest]
#[tokio::test]
async fn database_test_update_batch(
    #[from(build_batch)]
    #[with(1, 100, 200)]
    batch: AggregatorBatch,
) {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    database_client.create_aggregator_batch(batch.clone()).await.unwrap();

    // Waiting for sometime to ensure updated_at is different after the update
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Create updates for the batch
    let updates = AggregatorBatchUpdates { end_block: Some(250), status: Some(AggregatorBatchStatus::Closed) };

    // Update the batch
    let updated_batch = database_client.update_or_create_aggregator_batch(&batch, &updates).await.unwrap();

    // Verify the updates
    assert_eq!(updated_batch.id, batch.id);
    assert_eq!(updated_batch.index, batch.index);
    // verify the block updates
    assert_eq!(updated_batch.num_blocks, updates.end_block.unwrap() - batch.start_block + 1);
    assert_eq!(updated_batch.start_block, batch.start_block);
    assert_eq!(updated_batch.end_block, updates.end_block.unwrap());
    // verify other updates
    assert_eq!(updated_batch.status, AggregatorBatchStatus::Closed);
    assert_eq!(updated_batch.bucket_id, batch.bucket_id);
    assert_eq!(updated_batch.squashed_state_updates_path, batch.squashed_state_updates_path);
    assert_eq!(updated_batch.blob_path, batch.blob_path);
    assert_eq!(updated_batch.created_at, batch.created_at);
    assert_ne!(updated_batch.updated_at, batch.updated_at);
}

#[rstest]
#[tokio::test]
async fn database_test_create_batch(
    #[from(build_batch)]
    #[with(1, 100, 200)]
    batch: AggregatorBatch,
) {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    // Create the batch
    let created_batch = database_client.create_aggregator_batch(batch.clone()).await.unwrap();

    // Verify the created batch matches the input
    assert_eq!(created_batch, batch);

    // Verify we can retrieve the batch
    let retrieved_batch = database_client.get_latest_aggregator_batch().await.unwrap().unwrap();
    assert_eq!(retrieved_batch, batch);
}

/// Test for `create_snos_batch` operation in database trait.
/// Creates a SNOS batch and verifies it can be retrieved correctly.
#[rstest]
#[tokio::test]
async fn test_create_snos_batch() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    // Create a SNOS batch
    let snos_batch = build_snos_batch(1, Some(100), 200);

    // Create the batch in the database
    let created_batch = database_client.create_snos_batch(snos_batch.clone()).await.unwrap();

    // Verify the created batch matches the input
    assert_eq!(created_batch, snos_batch);

    // Verify we can retrieve the batch by getting the latest SNOS batch
    let retrieved_batch = database_client.get_latest_snos_batch().await.unwrap().unwrap();
    assert_eq!(retrieved_batch, snos_batch);

    // Verify batch properties
    assert_eq!(retrieved_batch.index, 1);
    assert_eq!(retrieved_batch.start_block, 200);
    assert_eq!(retrieved_batch.end_block, 200);
    assert_eq!(retrieved_batch.num_blocks, 1); // 200 - 100 + 1
    assert_eq!(retrieved_batch.status, SnosBatchStatus::Open);
}

/// Test for `get_job_by_id` operation in database trait.
/// Creates a job and retrieves it by its UUID.
#[rstest]
#[tokio::test]
async fn test_get_job_by_id() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    let job = build_job_item(JobType::SnosRun, JobStatus::Created, 1);
    let job_id = job.id;

    database_client.create_job(job.clone()).await.unwrap();

    let retrieved_job = database_client.get_job_by_id(job_id).await.unwrap().unwrap();
    assert_eq!(retrieved_job, job);

    // Test non-existent job
    let non_existent_id = uuid::Uuid::new_v4();
    let result = database_client.get_job_by_id(non_existent_id).await.unwrap();
    assert!(result.is_none());
}

/// Test for `get_jobs_by_types_and_statuses` operation in database trait.
/// Creates multiple jobs with different types and statuses.
#[rstest]
#[tokio::test]
async fn test_get_jobs_by_types_and_statuses() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    let jobs = [
        build_job_item(JobType::SnosRun, JobStatus::Completed, 1),
        build_job_item(JobType::ProofCreation, JobStatus::Completed, 2),
        build_job_item(JobType::SnosRun, JobStatus::Created, 3),
        build_job_item(JobType::ProofCreation, JobStatus::Failed, 4),
        build_job_item(JobType::DataSubmission, JobStatus::Completed, 5),
    ];

    for job in &jobs {
        database_client.create_job(job.clone()).await.unwrap();
    }

    // Test with multiple types and statuses
    let retrieved_jobs = database_client
        .get_jobs_by_types_and_statuses(
            vec![JobType::SnosRun, JobType::ProofCreation],
            vec![JobStatus::Completed],
            None,
        )
        .await
        .unwrap();

    assert_eq!(retrieved_jobs.len(), 2);
    assert!(retrieved_jobs.contains(&jobs[0]));
    assert!(retrieved_jobs.contains(&jobs[1]));

    // Test with limit
    let limited_jobs = database_client
        .get_jobs_by_types_and_statuses(
            vec![JobType::SnosRun, JobType::ProofCreation],
            vec![JobStatus::Completed],
            Some(1),
        )
        .await
        .unwrap();

    assert_eq!(limited_jobs.len(), 1);
}

/// Test for `get_jobs_between_internal_ids` operation in database trait.
/// Creates jobs and retrieves those within a range of internal IDs.
#[rstest]
#[tokio::test]
async fn test_get_jobs_between_internal_ids() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    let jobs = [
        build_job_item(JobType::SnosRun, JobStatus::Completed, 1),
        build_job_item(JobType::SnosRun, JobStatus::Completed, 2),
        build_job_item(JobType::SnosRun, JobStatus::Completed, 3),
        build_job_item(JobType::SnosRun, JobStatus::Completed, 4),
        build_job_item(JobType::SnosRun, JobStatus::Completed, 5),
    ];

    for job in &jobs {
        database_client.create_job(job.clone()).await.unwrap();
    }

    let retrieved_jobs =
        database_client.get_jobs_between_internal_ids(JobType::SnosRun, JobStatus::Completed, 2, 4).await.unwrap();

    assert_eq!(retrieved_jobs.len(), 3);
    assert_eq!(retrieved_jobs[0], jobs[1]);
    assert_eq!(retrieved_jobs[1], jobs[2]);
    assert_eq!(retrieved_jobs[2], jobs[3]);
}

/// Test for `get_jobs_by_type_and_statuses` operation in database trait.
/// Creates jobs with different statuses and retrieves by type and multiple statuses.
#[rstest]
#[tokio::test]
async fn test_get_jobs_by_type_and_statuses() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    let jobs = [
        build_job_item(JobType::SnosRun, JobStatus::Completed, 1),
        build_job_item(JobType::SnosRun, JobStatus::Created, 2),
        build_job_item(JobType::SnosRun, JobStatus::Failed, 3),
        build_job_item(JobType::ProofCreation, JobStatus::Completed, 4),
    ];

    for job in &jobs {
        database_client.create_job(job.clone()).await.unwrap();
    }

    let retrieved_jobs = database_client
        .get_jobs_by_type_and_statuses(&JobType::SnosRun, vec![JobStatus::Completed, JobStatus::Failed])
        .await
        .unwrap();

    assert_eq!(retrieved_jobs.len(), 2);
    assert!(retrieved_jobs.contains(&jobs[0]));
    assert!(retrieved_jobs.contains(&jobs[2]));
}

/// Test for `get_jobs_by_block_number` operation in database trait.
/// Creates jobs with different block numbers and retrieves by block number.
#[rstest]
#[tokio::test]
async fn test_get_jobs_by_block_number() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    // Create jobs with block number 100
    let jobs = [
        build_job_item(JobType::SnosRun, JobStatus::Completed, 100),
        build_job_item(JobType::ProofCreation, JobStatus::Created, 100),
        build_job_item(JobType::SnosRun, JobStatus::Completed, 200),
    ];

    for job in &jobs {
        database_client.create_job(job.clone()).await.unwrap();
    }

    let retrieved_jobs = database_client.get_jobs_by_block_number(100).await.unwrap();

    assert_eq!(retrieved_jobs.len(), 2);
    assert!(retrieved_jobs.contains(&jobs[0]));
    assert!(retrieved_jobs.contains(&jobs[1]));
}

/// Test for `get_orphaned_jobs` operation in database trait.
/// Creates jobs stuck in LockedForProcessing status and retrieves orphaned ones.
#[rstest]
#[tokio::test]
async fn test_get_orphaned_jobs() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    let jobs = [
        build_job_item(JobType::SnosRun, JobStatus::LockedForProcessing, 1),
        build_job_item(JobType::SnosRun, JobStatus::Created, 2),
        build_job_item(JobType::SnosRun, JobStatus::LockedForProcessing, 3),
    ];

    for mut job in jobs {
        job.metadata.common.process_started_at = Some(Utc::now());
        database_client.create_job(job.clone()).await.unwrap();
    }

    // Wait for jobs to become orphaned
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    let orphaned_jobs = database_client.get_orphaned_jobs(&JobType::SnosRun, 3).await.unwrap();

    assert_eq!(orphaned_jobs.len(), 2);
    assert!(orphaned_jobs.iter().any(|j| j.internal_id == 1));
    assert!(orphaned_jobs.iter().any(|j| j.internal_id == 3));
}

/// Test for `get_snos_batches_by_indices` operation in database trait.
/// Creates multiple SNOS batches and retrieves by their indices.
#[rstest]
#[tokio::test]
async fn test_get_snos_batches_by_indices() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    let batches =
        [build_snos_batch(1, Some(100), 200), build_snos_batch(2, Some(100), 300), build_snos_batch(3, Some(100), 400)];

    for batch in &batches {
        database_client.create_snos_batch(batch.clone()).await.unwrap();
    }

    let retrieved_batches = database_client.get_snos_batches_by_indices(vec![1, 3]).await.unwrap();

    println!("{:?}", retrieved_batches);

    assert_eq!(retrieved_batches.len(), 2);
    assert_eq!(retrieved_batches[0].index, 1);
    assert_eq!(retrieved_batches[1].index, 3);
}

/// Test for `update_snos_batch_status_by_index` operation in database trait.
/// Creates a SNOS batch and updates its status.
#[rstest]
#[tokio::test]
async fn test_update_snos_batch_status_by_index() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    let batch = build_snos_batch(1, Some(100), 200);
    database_client.create_snos_batch(batch.clone()).await.unwrap();

    let updated_batch = database_client.update_snos_batch_status_by_index(1, SnosBatchStatus::Closed).await.unwrap();

    assert_eq!(updated_batch.status, SnosBatchStatus::Closed);
    assert_eq!(updated_batch.index, 1);
}

/// Test for `get_snos_batches_by_status` operation in database trait.
/// Creates SNOS batches with different statuses and versions, tests filtering.
#[rstest]
#[tokio::test]
async fn test_get_snos_batches_by_status() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    let mut batch1 = build_snos_batch(1, Some(100), 200);
    let batch2 = build_snos_batch(2, Some(100), 300);
    let mut batch3 = build_snos_batch_with_version(3, Some(100), 400, "old-version".to_string());

    batch1.status = SnosBatchStatus::Closed;
    batch3.status = SnosBatchStatus::Closed;

    database_client.create_snos_batch(batch1.clone()).await.unwrap();
    database_client.create_snos_batch(batch2.clone()).await.unwrap();
    database_client.create_snos_batch(batch3.clone()).await.unwrap();

    // Test without version filter - should return all closed batches
    let closed_batches = database_client.get_snos_batches_by_status(SnosBatchStatus::Closed, None, None).await.unwrap();
    assert_eq!(closed_batches.len(), 2);
    assert!(closed_batches.iter().any(|b| b.index == 1));
    assert!(closed_batches.iter().any(|b| b.index == 3));

    // Test with current version filter - should only return batch 1
    let current_version = crate::types::constant::ORCHESTRATOR_VERSION.to_string();
    let current_version_batches = database_client
        .get_snos_batches_by_status(SnosBatchStatus::Closed, None, Some(current_version))
        .await
        .unwrap();
    assert_eq!(current_version_batches.len(), 1);
    assert_eq!(current_version_batches[0].index, 1);

    // Test with old version filter - should only return batch 3
    let old_version_batches = database_client
        .get_snos_batches_by_status(SnosBatchStatus::Closed, None, Some("old-version".to_string()))
        .await
        .unwrap();
    assert_eq!(old_version_batches.len(), 1);
    assert_eq!(old_version_batches[0].index, 3);

    // Test with limit
    let limited_batches =
        database_client.get_snos_batches_by_status(SnosBatchStatus::Closed, Some(1), None).await.unwrap();
    assert_eq!(limited_batches.len(), 1);
}

/// Test for `get_snos_batches_without_jobs` operation in database trait.
/// Creates SNOS batches with different versions and checks for those without corresponding jobs.
#[rstest]
#[tokio::test]
async fn test_get_snos_batches_without_jobs() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    let mut batch1 = build_snos_batch(1, Some(100), 200);
    let mut batch2 = build_snos_batch(2, Some(100), 300);
    let mut batch3 = build_snos_batch_with_version(3, Some(100), 400, "old-version".to_string());
    batch1.status = SnosBatchStatus::Closed;
    batch2.status = SnosBatchStatus::Closed;
    batch3.status = SnosBatchStatus::Closed;

    database_client.create_snos_batch(batch1.clone()).await.unwrap();
    database_client.create_snos_batch(batch2.clone()).await.unwrap();
    database_client.create_snos_batch(batch3.clone()).await.unwrap();

    // Create a job for batch 1 only
    let job = build_job_item(JobType::SnosRun, JobStatus::Created, 1);
    database_client.create_job(job).await.unwrap();

    // Test without version filter - should return batches 2 and 3 (no jobs)
    let batches_without_jobs =
        database_client.get_snos_batches_without_jobs(SnosBatchStatus::Closed, 5, None).await.unwrap();
    assert_eq!(batches_without_jobs.len(), 2);
    assert!(batches_without_jobs.iter().any(|b| b.index == 2));
    assert!(batches_without_jobs.iter().any(|b| b.index == 3));

    // Test with current version filter - should only return batch 2
    let current_version = crate::types::constant::ORCHESTRATOR_VERSION.to_string();
    let current_version_batches = database_client
        .get_snos_batches_without_jobs(SnosBatchStatus::Closed, 5, Some(current_version))
        .await
        .unwrap();
    assert_eq!(current_version_batches.len(), 1);
    assert_eq!(current_version_batches[0].index, 2);

    // Test with old version filter - should only return batch 3
    let old_version_batches = database_client
        .get_snos_batches_without_jobs(SnosBatchStatus::Closed, 5, Some("old-version".to_string()))
        .await
        .unwrap();
    assert_eq!(old_version_batches.len(), 1);
    assert_eq!(old_version_batches[0].index, 3);
}

/// Test for `get_aggregator_batches_by_indexes` operation in database trait.
/// Creates multiple aggregator batches and retrieves by their indexes.
#[rstest]
#[tokio::test]
async fn test_get_aggregator_batches_by_indexes(
    #[from(build_batch)]
    #[with(1, 100, 200)]
    batch1: AggregatorBatch,
    #[from(build_batch)]
    #[with(2, 210, 300)]
    batch2: AggregatorBatch,
    #[from(build_batch)]
    #[with(3, 301, 400)]
    batch3: AggregatorBatch,
) {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    database_client.create_aggregator_batch(batch1.clone()).await.unwrap();
    database_client.create_aggregator_batch(batch2.clone()).await.unwrap();
    database_client.create_aggregator_batch(batch3.clone()).await.unwrap();

    let retrieved_batches = database_client.get_aggregator_batches_by_indexes(vec![1, 3]).await.unwrap();

    assert_eq!(retrieved_batches.len(), 2);
    assert_eq!(retrieved_batches[0].index, 1);
    assert_eq!(retrieved_batches[1].index, 3);
}

/// Test for `update_aggregator_batch_status_by_index` operation in database trait.
/// Creates an aggregator batch and updates its status.
#[rstest]
#[tokio::test]
async fn test_update_aggregator_batch_status_by_index(
    #[from(build_batch)]
    #[with(1, 100, 200)]
    batch: AggregatorBatch,
) {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    database_client.create_aggregator_batch(batch.clone()).await.unwrap();

    let updated_batch =
        database_client.update_aggregator_batch_status_by_index(1, AggregatorBatchStatus::Closed).await.unwrap();

    assert_eq!(updated_batch.status, AggregatorBatchStatus::Closed);
    assert_eq!(updated_batch.index, 1);
}

/// Test for `get_aggregator_batch_for_block` operation in database trait.
/// Creates aggregator batches and retrieves the one containing a specific block.
#[rstest]
#[tokio::test]
async fn test_get_aggregator_batch_for_block(
    #[from(build_batch)]
    #[with(1, 100, 200)]
    batch1: AggregatorBatch,
    #[from(build_batch)]
    #[with(2, 201, 300)]
    batch2: AggregatorBatch,
) {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    database_client.create_aggregator_batch(batch1.clone()).await.unwrap();
    database_client.create_aggregator_batch(batch2.clone()).await.unwrap();

    let retrieved_batch = database_client.get_aggregator_batch_for_block(150).await.unwrap().unwrap();

    assert_eq!(retrieved_batch.index, 1);
    assert_eq!(retrieved_batch.start_block, 100);
    assert_eq!(retrieved_batch.end_block, 200);

    let retrieved_batch2 = database_client.get_aggregator_batch_for_block(250).await.unwrap().unwrap();

    assert_eq!(retrieved_batch2.index, 2);
}

/// Test for `get_aggregator_batches_by_status` operation in database trait.
/// Creates aggregator batches with different statuses and versions, tests filtering.
#[rstest]
#[tokio::test]
async fn test_get_aggregator_batches_by_status() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    let current_version = crate::types::constant::ORCHESTRATOR_VERSION.to_string();
    let mut batch1 = build_batch_with_version(1, 100, 200, current_version.clone());
    let batch2 = build_batch_with_version(2, 201, 300, current_version.clone());
    let mut batch3 = build_batch_with_version(3, 301, 400, "old-version".to_string());

    batch1.status = AggregatorBatchStatus::Closed;
    batch3.status = AggregatorBatchStatus::Closed;

    database_client.create_aggregator_batch(batch1.clone()).await.unwrap();
    database_client.create_aggregator_batch(batch2.clone()).await.unwrap();
    database_client.create_aggregator_batch(batch3.clone()).await.unwrap();

    // Test without version filter - should return all closed batches
    let closed_batches =
        database_client.get_aggregator_batches_by_status(AggregatorBatchStatus::Closed, None, None).await.unwrap();
    assert_eq!(closed_batches.len(), 2);
    assert!(closed_batches.iter().any(|b| b.index == 1));
    assert!(closed_batches.iter().any(|b| b.index == 3));

    // Test with current version filter - should only return batch 1
    let current_version_batches = database_client
        .get_aggregator_batches_by_status(AggregatorBatchStatus::Closed, None, Some(current_version))
        .await
        .unwrap();
    assert_eq!(current_version_batches.len(), 1);
    assert_eq!(current_version_batches[0].index, 1);

    // Test with old version filter - should only return batch 3
    let old_version_batches = database_client
        .get_aggregator_batches_by_status(AggregatorBatchStatus::Closed, None, Some("old-version".to_string()))
        .await
        .unwrap();
    assert_eq!(old_version_batches.len(), 1);
    assert_eq!(old_version_batches[0].index, 3);

    // Test with limit
    let limited_batches =
        database_client.get_aggregator_batches_by_status(AggregatorBatchStatus::Closed, Some(1), None).await.unwrap();
    assert_eq!(limited_batches.len(), 1);
}

/// Test for `get_snos_batches_by_aggregator_index` operation in database trait.
/// Creates SNOS batches belonging to different aggregator batches.
#[rstest]
#[tokio::test]
async fn test_get_snos_batches_by_aggregator_index() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    let mut batch1 = build_snos_batch(1, Some(1), 200);
    let mut batch2 = build_snos_batch(2, Some(1), 300);
    let mut batch3 = build_snos_batch(3, Some(2), 400);

    batch1.aggregator_batch_index = Some(1);
    batch2.aggregator_batch_index = Some(1);
    batch3.aggregator_batch_index = Some(2);

    database_client.create_snos_batch(batch1.clone()).await.unwrap();
    database_client.create_snos_batch(batch2.clone()).await.unwrap();
    database_client.create_snos_batch(batch3.clone()).await.unwrap();

    let batches_for_agg1 = database_client.get_snos_batches_by_aggregator_index(1).await.unwrap();

    assert_eq!(batches_for_agg1.len(), 2);
    assert!(batches_for_agg1.iter().any(|b| b.index == 1));
    assert!(batches_for_agg1.iter().any(|b| b.index == 2));
}

/// Test for `get_open_snos_batches_by_aggregator_index` operation in database trait.
/// Creates SNOS batches with different statuses for an aggregator batch.
#[rstest]
#[tokio::test]
async fn test_get_open_snos_batches_by_aggregator_index() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    let mut batch1 = build_snos_batch(1, Some(1), 200);
    let mut batch2 = build_snos_batch(2, Some(1), 300);
    let mut batch3 = build_snos_batch(3, Some(1), 400);

    batch1.aggregator_batch_index = Some(1);
    batch2.aggregator_batch_index = Some(1);
    batch3.aggregator_batch_index = Some(1);
    batch2.status = SnosBatchStatus::Closed;

    database_client.create_snos_batch(batch1.clone()).await.unwrap();
    database_client.create_snos_batch(batch2.clone()).await.unwrap();
    database_client.create_snos_batch(batch3.clone()).await.unwrap();

    let open_batches = database_client.get_open_snos_batches_by_aggregator_index(1).await.unwrap();

    assert_eq!(open_batches.len(), 2);
    assert!(open_batches.iter().any(|b| b.index == 1));
    assert!(open_batches.iter().any(|b| b.index == 3));
}

/// Test for `get_next_snos_batch_id` operation in database trait.
/// Creates SNOS batches and verifies next ID calculation.
#[rstest]
#[tokio::test]
async fn test_get_next_snos_batch_id() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    // Initially, should return 1
    let next_id = database_client.get_next_snos_batch_id().await.unwrap();
    assert_eq!(next_id, 1);

    // Create some batches
    database_client.create_snos_batch(build_snos_batch(1, Some(100), 200)).await.unwrap();
    database_client.create_snos_batch(build_snos_batch(2, Some(100), 300)).await.unwrap();

    // Should now return 3
    let next_id = database_client.get_next_snos_batch_id().await.unwrap();
    assert_eq!(next_id, 3);
}

/// Test for `close_all_snos_batches_for_aggregator` operation in database trait.
/// Creates open SNOS batches for an aggregator and closes them all.
#[rstest]
#[tokio::test]
async fn test_close_all_snos_batches_for_aggregator() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    let mut batch1 = build_snos_batch(1, Some(1), 200);
    let mut batch2 = build_snos_batch(2, Some(1), 300);
    let mut batch3 = build_snos_batch(3, Some(2), 400);

    batch1.aggregator_batch_index = Some(1);
    batch2.aggregator_batch_index = Some(1);
    batch3.aggregator_batch_index = Some(2);

    database_client.create_snos_batch(batch1.clone()).await.unwrap();
    database_client.create_snos_batch(batch2.clone()).await.unwrap();
    database_client.create_snos_batch(batch3.clone()).await.unwrap();

    let closed_batches = database_client.close_all_snos_batches_for_aggregator(1).await.unwrap();

    assert_eq!(closed_batches.len(), 2);
    assert!(closed_batches.iter().all(|b| b.status == SnosBatchStatus::Closed));
    assert!(closed_batches.iter().any(|b| b.index == 1));
    assert!(closed_batches.iter().any(|b| b.index == 2));

    // Verify batch 3 is still open
    let batch3_after = database_client.get_snos_batches_by_indices(vec![3]).await.unwrap();
    assert_eq!(batch3_after[0].status, SnosBatchStatus::Open);
}

/// Test for `get_oldest_aggregator_batch` operation in database trait.
/// Creates aggregator batches with different versions and verifies retrieval.
#[rstest]
#[tokio::test]
async fn test_get_oldest_aggregator_batch() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    // Create older batch with old version (index 1, starts at block 100)
    let batch_old = build_batch_with_version(1, 100, 200, "old-version".to_string());
    // Create newer batch with current version (index 2, starts at block 201)
    let batch_current = build_batch_with_version(
        2,
        201,
        300,
        crate::types::constant::ORCHESTRATOR_VERSION.to_string(),
    );

    database_client.create_aggregator_batch(batch_old.clone()).await.unwrap();
    database_client.create_aggregator_batch(batch_current.clone()).await.unwrap();

    // Query without version filter - should get the truly oldest batch (index 1)
    let oldest_all = database_client.get_oldest_aggregator_batch(None).await.unwrap();
    assert!(oldest_all.is_some(), "Should find any batch");
    assert_eq!(oldest_all.unwrap().index, 1, "Should return the oldest batch regardless of version");

    // Query with current version filter - should get the newer batch (only one with current version)
    let current_version = crate::types::constant::ORCHESTRATOR_VERSION.to_string();
    let oldest_current = database_client.get_oldest_aggregator_batch(Some(current_version)).await.unwrap();
    assert!(oldest_current.is_some(), "Should find batch with current version");
    assert_eq!(oldest_current.unwrap().index, 2, "Should return the batch with current version");

    // Query with old version filter - should get the older batch
    let oldest_old = database_client.get_oldest_aggregator_batch(Some("old-version".to_string())).await.unwrap();
    assert!(oldest_old.is_some(), "Should find batch with old version");
    assert_eq!(oldest_old.unwrap().index, 1, "Should return the batch with old version");
}
