// Storage Cleanup Worker Tests
// Unit tests with mocks for orchestration logic + Integration test with LocalStack/MongoDB

use bytes::Bytes;
use rstest::rstest;
use std::error::Error;
use std::sync::Arc;

use crate::core::client::database::{DatabaseError, MockDatabaseClient};
use crate::core::client::lock::error::LockError;
use crate::core::client::lock::{LockResult, MockLockClient};
use crate::core::client::storage::MockStorageClient;
use crate::tests::config::{ConfigType, TestConfigBuilder};
use crate::tests::workers::utils::get_job_by_mock_id_vector;
use crate::types::constant::{
    get_batch_artifact_file, get_batch_blob_file, get_batch_state_update_file, get_snos_legacy_dir,
    STORAGE_EXPIRATION_TAG_KEY, STORAGE_EXPIRATION_TAG_VALUE,
};
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{
    CommonMetadata, JobMetadata, JobSpecificMetadata, SettlementContext, SettlementContextData, StateUpdateMetadata,
};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::worker::event_handler::triggers::storage_cleanup::StorageCleanupTrigger;
use crate::worker::event_handler::triggers::JobTrigger;

// =============================================================================
// Unit Tests with Mocks
// =============================================================================

/// Test: Worker skips processing when lock is unavailable (another instance running)
#[rstest]
#[tokio::test]
async fn test_run_worker_skips_when_lock_unavailable() -> Result<(), Box<dyn Error>> {
    let database = MockDatabaseClient::new();
    let storage = MockStorageClient::new();
    let mut lock = MockLockClient::new();

    lock.expect_acquire_lock()
        .returning(|_, _, _, _| Err(LockError::LockAlreadyHeld { current_owner: "other_instance".to_string() }));

    let services = TestConfigBuilder::new()
        .configure_storage_client(storage.into())
        .configure_database(database.into())
        .configure_lock_client(lock.into())
        .build()
        .await;

    let result = StorageCleanupTrigger.run_worker(services.config).await;
    assert!(result.is_ok(), "Worker should gracefully exit when lock unavailable");
    Ok(())
}

/// Test: Lock is released even when processing fails (prevents deadlocks)
#[rstest]
#[tokio::test]
async fn test_run_worker_releases_lock_on_error() -> Result<(), Box<dyn Error>> {
    let mut database = MockDatabaseClient::new();
    let storage = MockStorageClient::new();
    let mut lock = MockLockClient::new();

    lock.expect_acquire_lock().returning(|_, _, _, _| Ok(LockResult::Acquired));
    database
        .expect_get_jobs_without_storage_artifacts_tagged()
        .returning(|_| Err(DatabaseError::KeyNotFound("simulated error".to_string())));
    lock.expect_release_lock().times(1).returning(|_, _| Ok(LockResult::Released));

    let services = TestConfigBuilder::new()
        .configure_storage_client(storage.into())
        .configure_database(database.into())
        .configure_lock_client(lock.into())
        .build()
        .await;

    let result = StorageCleanupTrigger.run_worker(services.config).await;
    assert!(result.is_err(), "Worker should return error when database fails");
    Ok(())
}

/// Test: Jobs with invalid metadata are skipped (one bad job doesn't stop batch)
#[rstest]
#[tokio::test]
async fn test_process_completed_jobs_skips_invalid_metadata() -> Result<(), Box<dyn Error>> {
    let mut database = MockDatabaseClient::new();
    let storage = MockStorageClient::new();
    let mut lock = MockLockClient::new();

    lock.expect_acquire_lock().returning(|_, _, _, _| Ok(LockResult::Acquired));
    lock.expect_release_lock().returning(|_, _| Ok(LockResult::Released));

    // Create job with wrong metadata type (DA instead of StateUpdate)
    let invalid_job = JobItem {
        id: uuid::Uuid::new_v4(),
        internal_id: 1,
        job_type: JobType::StateTransition,
        status: JobStatus::Completed,
        external_id: crate::types::jobs::external_id::ExternalId::Number(1),
        metadata: JobMetadata {
            common: CommonMetadata::default(),
            specific: JobSpecificMetadata::Da(crate::types::jobs::metadata::DaMetadata {
                block_number: 1,
                blob_data_path: None,
                tx_hash: None,
            }),
        },
        version: 0,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    database.expect_get_jobs_without_storage_artifacts_tagged().returning(move |_| Ok(vec![invalid_job.clone()]));

    let services = TestConfigBuilder::new()
        .configure_storage_client(storage.into())
        .configure_database(database.into())
        .configure_lock_client(lock.into())
        .build()
        .await;

    let result = StorageCleanupTrigger.run_worker(services.config).await;
    assert!(result.is_ok(), "Worker should skip invalid jobs and continue");
    Ok(())
}

/// Test: Partial tagging failures don't mark job as tagged (retry next run)
#[rstest]
#[tokio::test]
async fn test_process_completed_jobs_skips_partial_tagging() -> Result<(), Box<dyn Error>> {
    let mut database = MockDatabaseClient::new();
    let mut storage = MockStorageClient::new();
    let mut lock = MockLockClient::new();

    lock.expect_acquire_lock().returning(|_, _, _, _| Ok(LockResult::Acquired));
    lock.expect_release_lock().returning(|_, _| Ok(LockResult::Released));

    let valid_jobs = get_job_by_mock_id_vector(JobType::StateTransition, JobStatus::Completed, 1, 1);
    let valid_job = valid_jobs.into_iter().next().expect("Should have created one job");

    database.expect_get_jobs_without_storage_artifacts_tagged().returning(move |_| Ok(vec![valid_job.clone()]));
    storage.expect_list_files_in_dir().returning(|_| Ok(vec!["file1.json".to_string()]));
    storage.expect_tag_object().returning(|_, _| {
        Err(crate::core::client::storage::StorageError::ObjectStreamError("Connection timeout".to_string()))
    });
    database.expect_update_job().never();

    let services = TestConfigBuilder::new()
        .configure_storage_client(storage.into())
        .configure_database(database.into())
        .configure_lock_client(lock.into())
        .build()
        .await;

    let result = StorageCleanupTrigger.run_worker(services.config).await;
    assert!(result.is_ok(), "Worker should continue even when tagging fails");
    Ok(())
}

// =============================================================================
// Integration Test with LocalStack + MongoDB
// =============================================================================

/// Full integration test: Creates S3 artifacts + MongoDB job, runs worker, verifies tagging
#[rstest]
#[tokio::test]
async fn test_storage_cleanup_happy_path_integration() -> Result<(), Box<dyn Error>> {
    let services = TestConfigBuilder::new()
        .configure_storage_client(ConfigType::Actual)
        .configure_database(ConfigType::Actual)
        .configure_lock_client(ConfigType::Actual)
        .build()
        .await;

    let config = &services.config;
    let job_id: u64 = 99999;
    let test_data = Bytes::from(r#"{"test": "happy_path_integration"}"#);

    // Step 1: Create artifacts in S3
    let artifact_file = get_batch_artifact_file(job_id, "proof.json");
    let blob_file = get_batch_blob_file(job_id, 0);
    let snos_file = format!("{}/snos_output.json", get_snos_legacy_dir(job_id));
    let state_update_file = get_batch_state_update_file(job_id);

    config.storage().put_data(test_data.clone(), &artifact_file).await?;
    config.storage().put_data(test_data.clone(), &blob_file).await?;
    config.storage().put_data(test_data.clone(), &snos_file).await?;
    config.storage().put_data(test_data.clone(), &state_update_file).await?;

    // Step 2: Create completed StateTransition job in MongoDB
    let job = JobItem {
        id: uuid::Uuid::new_v4(),
        internal_id: job_id,
        job_type: JobType::StateTransition,
        status: JobStatus::Completed,
        external_id: crate::types::jobs::external_id::ExternalId::Number(job_id as usize),
        metadata: JobMetadata {
            common: CommonMetadata::default(),
            specific: JobSpecificMetadata::StateUpdate(StateUpdateMetadata {
                snos_output_path: Some(snos_file.clone()),
                program_output_path: None,
                blob_data_path: None,
                da_segment_path: None,
                tx_hash: Some("0x123".to_string()),
                context: SettlementContext::Batch(SettlementContextData { to_settle: job_id, last_failed: None }),
                storage_artifacts_tagged_at: None,
            }),
        },
        version: 0,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    config.database().create_job(job.clone()).await?;

    // Verify job is in untagged list
    let untagged_jobs = config.database().get_jobs_without_storage_artifacts_tagged(Some(100)).await?;
    assert!(untagged_jobs.iter().any(|j| j.internal_id == job_id), "Job should be in untagged list");

    // Step 3: Run StorageCleanupTrigger
    let result = StorageCleanupTrigger.run_worker(config.clone()).await;
    assert!(result.is_ok(), "StorageCleanupTrigger should complete: {:?}", result.err());

    // Step 4: Verify artifacts are tagged
    let bucket_name = services.storage_params.bucket_identifier.to_string();
    let artifact_tags = get_object_tags(&services.provider_config, &bucket_name, &artifact_file).await?;
    assert!(
        artifact_tags.iter().any(|(k, v)| k == STORAGE_EXPIRATION_TAG_KEY && v == STORAGE_EXPIRATION_TAG_VALUE),
        "Artifact should have expiration tag: {:?}",
        artifact_tags
    );

    // Step 5: Verify job is no longer in untagged list
    let untagged_jobs_after = config.database().get_jobs_without_storage_artifacts_tagged(Some(100)).await?;
    assert!(!untagged_jobs_after.iter().any(|j| j.internal_id == job_id), "Job should be marked as tagged");

    Ok(())
}

// =============================================================================
// S3 Storage Client Unit Tests
// =============================================================================

/// Tests: tag_object applies tags correctly + tag_nonexistent_object returns error
#[rstest]
#[tokio::test]
async fn test_tag_object_success_and_error_cases() -> Result<(), Box<dyn Error>> {
    let services = TestConfigBuilder::new().configure_storage_client(ConfigType::Actual).build().await;
    let storage = services.config.storage();

    // Setup: create a test object
    let test_key = "test_tagging/object.json";
    storage.put_data(Bytes::from(r#"{"test": "tagging"}"#), test_key).await?;

    // Test 1: tag_object applies tags correctly
    let tags = [(STORAGE_EXPIRATION_TAG_KEY.to_string(), STORAGE_EXPIRATION_TAG_VALUE.to_string())];
    storage.tag_object(test_key, &tags).await?;

    let bucket_name = services.storage_params.bucket_identifier.to_string();
    let applied_tags = get_object_tags(&services.provider_config, &bucket_name, test_key).await?;
    assert!(
        applied_tags.iter().any(|(k, v)| k == STORAGE_EXPIRATION_TAG_KEY && v == STORAGE_EXPIRATION_TAG_VALUE),
        "Tag should be applied: {:?}",
        applied_tags
    );

    // Test 2: tag_nonexistent_object returns error
    let result = storage.tag_object("nonexistent/path/file.json", &tags).await;
    assert!(result.is_err(), "Tagging nonexistent object should return error");

    Ok(())
}

/// Tests: list_files_in_dir returns all files in directory
#[rstest]
#[tokio::test]
async fn test_list_files_in_dir_returns_all_files() -> Result<(), Box<dyn Error>> {
    let services = TestConfigBuilder::new().configure_storage_client(ConfigType::Actual).build().await;
    let storage = services.config.storage();

    // Setup: create multiple files in a directory
    let test_dir = "test_list_dir_12345";
    let files = ["file1.json", "file2.json", "nested/file3.json"];
    for file in &files {
        let key = format!("{}/{}", test_dir, file);
        storage.put_data(Bytes::from(r#"{"test": "list"}"#), &key).await?;
    }

    // Test: list_files_in_dir returns all files
    let listed_files = storage.list_files_in_dir(test_dir).await?;
    assert_eq!(listed_files.len(), 3, "Should list all 3 files");
    for file in &files {
        let expected_key = format!("{}/{}", test_dir, file);
        assert!(listed_files.contains(&expected_key), "Should contain {}", expected_key);
    }

    // Test: empty directory returns empty list
    let empty_result = storage.list_files_in_dir("nonexistent_dir_xyz").await?;
    assert!(empty_result.is_empty(), "Nonexistent dir should return empty list");

    Ok(())
}

/// Tests: setup_lifecycle_rule creates correct configuration
#[rstest]
#[tokio::test]
async fn test_setup_lifecycle_rule_creates_correct_config() -> Result<(), Box<dyn Error>> {
    use crate::types::constant::STORAGE_LIFECYCLE_RULE_ID;

    let services = TestConfigBuilder::new().configure_storage_client(ConfigType::Actual).build().await;
    let bucket_name = services.storage_params.bucket_identifier.to_string();
    let aws_config = services.provider_config.get_aws_client_or_panic();

    let mut s3_config_builder = aws_sdk_s3::config::Builder::from(aws_config);
    s3_config_builder.set_force_path_style(Some(true));
    let client = aws_sdk_s3::Client::from_conf(s3_config_builder.build());

    // Lifecycle rule is created during TestConfigBuilder setup, verify it exists
    let lifecycle = client.get_bucket_lifecycle_configuration().bucket(&bucket_name).send().await?;
    let rules = lifecycle.rules();

    let cleanup_rule = rules.iter().find(|r| r.id() == Some(STORAGE_LIFECYCLE_RULE_ID));
    assert!(cleanup_rule.is_some(), "Lifecycle rule should exist");

    let rule = cleanup_rule.unwrap();
    assert_eq!(*rule.status(), aws_sdk_s3::types::ExpirationStatus::Enabled);
    assert!(rule.expiration().is_some(), "Rule should have expiration configured");

    // Verify tag filter
    if let Some(filter) = rule.filter() {
        if let Some(tag) = filter.tag() {
            assert_eq!(tag.key(), STORAGE_EXPIRATION_TAG_KEY);
            assert_eq!(tag.value(), STORAGE_EXPIRATION_TAG_VALUE);
        } else {
            panic!("Rule should have tag filter");
        }
    }

    Ok(())
}

/// Tests: collect_and_tag_artifacts integration (full worker flow for single job)
#[rstest]
#[tokio::test]
async fn test_collect_and_tag_artifacts_integration() -> Result<(), Box<dyn Error>> {
    let services = TestConfigBuilder::new()
        .configure_storage_client(ConfigType::Actual)
        .configure_database(ConfigType::Actual)
        .configure_lock_client(ConfigType::Actual)
        .build()
        .await;

    let config = &services.config;
    let job_id: u64 = 88888;
    let test_data = Bytes::from(r#"{"test": "collect_and_tag"}"#);

    // Create artifacts in multiple directories
    let artifact_file = get_batch_artifact_file(job_id, "test_proof.json");
    let blob_file = get_batch_blob_file(job_id, 0);
    let state_update_file = get_batch_state_update_file(job_id);

    config.storage().put_data(test_data.clone(), &artifact_file).await?;
    config.storage().put_data(test_data.clone(), &blob_file).await?;
    config.storage().put_data(test_data.clone(), &state_update_file).await?;

    // Create job in database
    let job = JobItem {
        id: uuid::Uuid::new_v4(),
        internal_id: job_id,
        job_type: JobType::StateTransition,
        status: JobStatus::Completed,
        external_id: crate::types::jobs::external_id::ExternalId::Number(job_id as usize),
        metadata: JobMetadata {
            common: CommonMetadata::default(),
            specific: JobSpecificMetadata::StateUpdate(StateUpdateMetadata {
                snos_output_path: None,
                program_output_path: None,
                blob_data_path: Some(blob_file.clone()),
                da_segment_path: None,
                tx_hash: Some("0xabc".to_string()),
                context: SettlementContext::Batch(SettlementContextData { to_settle: job_id, last_failed: None }),
                storage_artifacts_tagged_at: None,
            }),
        },
        version: 0,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    config.database().create_job(job).await?;

    // Run worker
    StorageCleanupTrigger.run_worker(config.clone()).await?;

    // Verify all artifacts are tagged
    let bucket_name = services.storage_params.bucket_identifier.to_string();
    for key in [&artifact_file, &blob_file, &state_update_file] {
        let tags = get_object_tags(&services.provider_config, &bucket_name, key).await?;
        assert!(
            tags.iter().any(|(k, v)| k == STORAGE_EXPIRATION_TAG_KEY && v == STORAGE_EXPIRATION_TAG_VALUE),
            "File {} should be tagged: {:?}",
            key,
            tags
        );
    }

    Ok(())
}

// =============================================================================
// Helper Functions
// =============================================================================

async fn get_object_tags(
    provider_config: &Arc<crate::core::cloud::CloudProvider>,
    bucket_name: &str,
    key: &str,
) -> Result<Vec<(String, String)>, Box<dyn Error>> {
    let aws_config = provider_config.get_aws_client_or_panic();
    let mut s3_config_builder = aws_sdk_s3::config::Builder::from(aws_config);
    s3_config_builder.set_force_path_style(Some(true));
    let client = aws_sdk_s3::Client::from_conf(s3_config_builder.build());

    let tagging_output = client.get_object_tagging().bucket(bucket_name).key(key).send().await?;
    Ok(tagging_output.tag_set().iter().map(|tag| (tag.key().to_string(), tag.value().to_string())).collect())
}
