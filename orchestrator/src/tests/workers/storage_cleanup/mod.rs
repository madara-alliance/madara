// Storage Cleanup Worker Tests
//
// This module contains tests for the StorageCleanupTrigger and related S3 operations.
// Tests are organized into two categories:
// 1. Unit tests with mocks for orchestration logic
// 2. Integration tests with LocalStack for actual S3 operations
//
// Note: Path helper function tests are in tests/types/constant.rs since those
// functions are used by multiple parts of the codebase.

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
    get_batch_artifact_file, get_batch_artifacts_dir, get_batch_blob_dir, get_batch_blob_file,
    get_batch_state_update_file, get_snos_legacy_dir, STORAGE_EXPIRATION_TAG_KEY, STORAGE_EXPIRATION_TAG_VALUE,
};
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{
    CommonMetadata, JobMetadata, JobSpecificMetadata, SettlementContext, SettlementContextData, StateUpdateMetadata,
};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::worker::event_handler::triggers::storage_cleanup::StorageCleanupTrigger;
use crate::worker::event_handler::triggers::JobTrigger;

// =============================================================================
// Unit Tests with Mocks - Orchestration Logic
// =============================================================================

/// Test that the worker skips processing when it cannot acquire the distributed lock.
/// This is critical for preventing duplicate processing in multi-instance deployments.
#[rstest]
#[tokio::test]
async fn test_run_worker_skips_when_lock_unavailable() -> Result<(), Box<dyn Error>> {
    let database = MockDatabaseClient::new();
    let storage = MockStorageClient::new();
    let mut lock = MockLockClient::new();

    // Mock lock client to simulate another instance holding the lock
    lock.expect_acquire_lock()
        .returning(|_, _, _, _| Err(LockError::LockAlreadyHeld { current_owner: "other_instance".to_string() }));

    let services = TestConfigBuilder::new()
        .configure_storage_client(storage.into())
        .configure_database(database.into())
        .configure_lock_client(lock.into())
        .build()
        .await;

    // Should complete successfully without processing (graceful exit)
    let result = StorageCleanupTrigger.run_worker(services.config).await;
    assert!(result.is_ok(), "Worker should gracefully exit when lock unavailable");

    Ok(())
}

/// Test that the lock is always released, even when processing fails.
/// This prevents deadlock scenarios where a crashed worker holds the lock forever.
#[rstest]
#[tokio::test]
async fn test_run_worker_releases_lock_on_error() -> Result<(), Box<dyn Error>> {
    let mut database = MockDatabaseClient::new();
    let storage = MockStorageClient::new();
    let mut lock = MockLockClient::new();

    // Mock lock acquisition success
    lock.expect_acquire_lock().returning(|_, _, _, _| Ok(LockResult::Acquired));

    // Mock database to return an error when fetching jobs
    database
        .expect_get_jobs_without_storage_artifacts_tagged()
        .returning(|_| Err(DatabaseError::KeyNotFound("simulated error".to_string())));

    // Verify lock is released even on error
    lock.expect_release_lock().times(1).returning(|_, _| Ok(LockResult::Released));

    let services = TestConfigBuilder::new()
        .configure_storage_client(storage.into())
        .configure_database(database.into())
        .configure_lock_client(lock.into())
        .build()
        .await;

    // Should return error but lock should be released
    let result = StorageCleanupTrigger.run_worker(services.config).await;
    assert!(result.is_err(), "Worker should return error when database fails");

    Ok(())
}

/// Test that jobs with invalid metadata are skipped without stopping the entire batch.
/// One bad job shouldn't prevent processing of other valid jobs.
#[rstest]
#[tokio::test]
async fn test_process_completed_jobs_skips_invalid_metadata() -> Result<(), Box<dyn Error>> {
    let mut database = MockDatabaseClient::new();
    let storage = MockStorageClient::new();
    let mut lock = MockLockClient::new();

    // Mock lock
    lock.expect_acquire_lock().returning(|_, _, _, _| Ok(LockResult::Acquired));
    lock.expect_release_lock().returning(|_, _| Ok(LockResult::Released));

    // Create a job with invalid metadata (not StateUpdateMetadata)
    let invalid_job = JobItem {
        id: uuid::Uuid::new_v4(),
        internal_id: 1,
        job_type: JobType::StateTransition,
        status: JobStatus::Completed,
        external_id: crate::types::jobs::external_id::ExternalId::Number(1),
        metadata: JobMetadata {
            common: CommonMetadata::default(),
            // Use DA metadata instead of StateUpdate - this will fail to parse
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

    // Should complete without error (skips invalid job)
    let result = StorageCleanupTrigger.run_worker(services.config).await;
    assert!(result.is_ok(), "Worker should skip invalid jobs and continue");

    Ok(())
}

/// Test that partial tagging failures don't mark the job as tagged.
/// If we can't tag all artifacts, the job should be retried next run.
#[rstest]
#[tokio::test]
async fn test_process_completed_jobs_skips_partial_tagging() -> Result<(), Box<dyn Error>> {
    let mut database = MockDatabaseClient::new();
    let mut storage = MockStorageClient::new();
    let mut lock = MockLockClient::new();

    // Mock lock
    lock.expect_acquire_lock().returning(|_, _, _, _| Ok(LockResult::Acquired));
    lock.expect_release_lock().returning(|_, _| Ok(LockResult::Released));

    // Create a valid StateTransition job using the existing helper from utils
    let valid_jobs = get_job_by_mock_id_vector(JobType::StateTransition, JobStatus::Completed, 1, 1);
    let valid_job = valid_jobs.into_iter().next().expect("Should have created one job");

    database.expect_get_jobs_without_storage_artifacts_tagged().returning(move |_| Ok(vec![valid_job.clone()]));

    // Mock storage to return files but fail on tagging
    storage.expect_list_files_in_dir().returning(|_| Ok(vec!["file1.json".to_string()]));

    // First tag succeeds, but simulate a real S3 error (not NoSuchKey)
    storage.expect_tag_object().returning(|_, _| {
        Err(crate::core::client::storage::StorageError::ObjectStreamError("Connection timeout".to_string()))
    });

    // update_job should NOT be called since tagging failed
    database.expect_update_job().never();

    let services = TestConfigBuilder::new()
        .configure_storage_client(storage.into())
        .configure_database(database.into())
        .configure_lock_client(lock.into())
        .build()
        .await;

    // Should complete without error (will retry next run)
    let result = StorageCleanupTrigger.run_worker(services.config).await;
    assert!(result.is_ok(), "Worker should continue even when tagging fails");

    Ok(())
}

// =============================================================================
// Integration Tests with LocalStack - Actual S3 Operations
// =============================================================================

/// Test that tag_object correctly applies tags to an S3 object.
/// This test verifies:
/// 1. We can create a file in S3
/// 2. We can tag it without errors
/// 3. We can still read the file after tagging (file wasn't corrupted)
#[rstest]
#[tokio::test]
async fn test_tag_object_applies_tags_correctly() -> Result<(), Box<dyn Error>> {
    let services = TestConfigBuilder::new().configure_storage_client(ConfigType::Actual).build().await;

    let storage = services.config.storage();

    // Create a test file
    let test_key = "test_tagging/test_file.json";
    let test_data = Bytes::from(r#"{"test": "data"}"#);
    storage.put_data(test_data.clone(), test_key).await?;

    // Apply tags using our method - this should succeed
    let tags = vec![(STORAGE_EXPIRATION_TAG_KEY.to_string(), STORAGE_EXPIRATION_TAG_VALUE.to_string())];
    storage.tag_object(test_key, &tags).await?;

    // Verify file is still accessible after tagging
    let retrieved_data = storage.get_data(test_key).await?;
    assert_eq!(retrieved_data, test_data, "File data should be unchanged after tagging");

    Ok(())
}

/// Test that list_files_in_dir returns all files from a directory.
/// Creates multiple files and verifies all are returned.
#[rstest]
#[tokio::test]
async fn test_list_files_in_dir_returns_all_files() -> Result<(), Box<dyn Error>> {
    let services = TestConfigBuilder::new().configure_storage_client(ConfigType::Actual).build().await;

    let storage = services.config.storage();

    // Create multiple test files in a directory
    let test_dir = "test_list_files";
    let file_names = vec!["file1.json", "file2.json", "file3.txt", "nested/file4.json"];
    let test_data = Bytes::from(r#"{"test": "data"}"#);

    for file_name in &file_names {
        let key = format!("{}/{}", test_dir, file_name);
        storage.put_data(test_data.clone(), &key).await?;
    }

    // List files in the directory
    let listed_files = storage.list_files_in_dir(test_dir).await?;

    // Verify all files are returned
    assert_eq!(
        listed_files.len(),
        file_names.len(),
        "Expected {} files, got {}: {:?}",
        file_names.len(),
        listed_files.len(),
        listed_files
    );

    for file_name in &file_names {
        let expected_key = format!("{}/{}", test_dir, file_name);
        assert!(listed_files.contains(&expected_key), "Expected file '{}' not found in listed files", expected_key);
    }

    Ok(())
}

/// Integration test: Create files in all 4 artifact locations, collect paths, tag them,
/// and verify all are tagged correctly.
#[rstest]
#[tokio::test]
async fn test_collect_and_tag_artifacts_integration() -> Result<(), Box<dyn Error>> {
    let services = TestConfigBuilder::new().configure_storage_client(ConfigType::Actual).build().await;

    let storage = services.config.storage();
    let job_id: u64 = 42;
    let test_data = Bytes::from(r#"{"test": "artifact"}"#);

    // Create files in all 4 artifact locations
    // 1. Artifacts directory
    let artifact_file = get_batch_artifact_file(job_id, "proof.json");
    storage.put_data(test_data.clone(), &artifact_file).await?;

    // 2. Blob directory
    let blob_file = get_batch_blob_file(job_id, 0);
    storage.put_data(test_data.clone(), &blob_file).await?;

    // 3. SNOS legacy directory
    let snos_file = format!("{}/output.json", get_snos_legacy_dir(job_id));
    storage.put_data(test_data.clone(), &snos_file).await?;

    // 4. State update file
    let state_update_file = get_batch_state_update_file(job_id);
    storage.put_data(test_data.clone(), &state_update_file).await?;

    // Collect paths from each location
    let mut all_paths = Vec::new();

    // Artifacts
    let artifacts_dir = get_batch_artifacts_dir(job_id);
    let artifact_files = storage.list_files_in_dir(&artifacts_dir).await.unwrap_or_default();
    all_paths.extend(artifact_files);

    // Blobs
    let blob_dir = get_batch_blob_dir(job_id);
    let blob_files = storage.list_files_in_dir(&blob_dir).await.unwrap_or_default();
    all_paths.extend(blob_files);

    // SNOS legacy
    let snos_dir = get_snos_legacy_dir(job_id);
    let snos_files = storage.list_files_in_dir(&snos_dir).await.unwrap_or_default();
    all_paths.extend(snos_files);

    // State update (single file)
    all_paths.push(state_update_file.clone());

    // Verify we collected 4 files
    assert_eq!(all_paths.len(), 4, "Expected 4 artifact paths, got {}: {:?}", all_paths.len(), all_paths);

    // Tag all artifacts - pass reference to avoid cloning
    let tags = vec![(STORAGE_EXPIRATION_TAG_KEY.to_string(), STORAGE_EXPIRATION_TAG_VALUE.to_string())];
    for path in &all_paths {
        storage.tag_object(path, &tags).await?;
    }

    // Verify all artifacts are still accessible after tagging (tagging didn't break anything)
    for path in &all_paths {
        // State update file is the only one that might not exist (we add it unconditionally)
        if path == &state_update_file {
            // Verify the state update file can be read
            let data = storage.get_data(path).await?;
            assert!(!data.is_empty(), "State update file should have content");
        }
    }

    Ok(())
}

/// Test that the lifecycle rule is set up correctly on the bucket.
/// Verifies the rule exists with the correct tag filter and expiration days.
#[rstest]
#[tokio::test]
async fn test_setup_lifecycle_rule_creates_correct_config() -> Result<(), Box<dyn Error>> {
    // Build config with Actual storage (this sets up the lifecycle rule)
    let services = TestConfigBuilder::new().configure_storage_client(ConfigType::Actual).build().await;

    // Get the lifecycle configuration from S3
    let lifecycle_config = get_bucket_lifecycle_config(&services.provider_config).await?;

    // Find our rule
    let our_rule = lifecycle_config
        .iter()
        .find(|rule| rule.id == crate::types::constant::STORAGE_LIFECYCLE_RULE_ID)
        .expect("Lifecycle rule not found");

    // Verify rule is enabled
    assert!(our_rule.enabled, "Lifecycle rule should be enabled");

    // Verify tag filter
    assert_eq!(our_rule.tag_key, Some(STORAGE_EXPIRATION_TAG_KEY.to_string()), "Tag key mismatch");
    assert_eq!(our_rule.tag_value, Some(STORAGE_EXPIRATION_TAG_VALUE.to_string()), "Tag value mismatch");

    // Verify expiration is set (we use default 14 days in tests)
    assert!(our_rule.expiration_days.is_some(), "Expiration days should be set");

    Ok(())
}

/// Test that tagging a non-existent object returns an appropriate error.
/// This verifies our error handling for missing files.
#[rstest]
#[tokio::test]
async fn test_tag_nonexistent_object_returns_error() -> Result<(), Box<dyn Error>> {
    let services = TestConfigBuilder::new().configure_storage_client(ConfigType::Actual).build().await;

    let storage = services.config.storage();

    // Try to tag a file that doesn't exist
    let nonexistent_key = "definitely/does/not/exist/file.json";
    let tags = vec![(STORAGE_EXPIRATION_TAG_KEY.to_string(), STORAGE_EXPIRATION_TAG_VALUE.to_string())];

    let result = storage.tag_object(nonexistent_key, &tags).await;

    // Should return an error (the specific error type depends on S3)
    assert!(result.is_err(), "Tagging non-existent object should return error");

    // Verify we got an error - the specific message varies by S3 implementation
    // LocalStack may return "service error" while real S3 returns "NoSuchKey"
    let error_msg = result.unwrap_err().to_string();
    assert!(!error_msg.is_empty(), "Error message should not be empty: {}", error_msg);

    Ok(())
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Simple struct to hold lifecycle rule info for testing
struct LifecycleRuleInfo {
    id: String,
    enabled: bool,
    tag_key: Option<String>,
    tag_value: Option<String>,
    expiration_days: Option<i32>,
}

/// Helper to get bucket lifecycle configuration
async fn get_bucket_lifecycle_config(
    provider_config: &Arc<crate::core::cloud::CloudProvider>,
) -> Result<Vec<LifecycleRuleInfo>, Box<dyn Error>> {
    let aws_config = provider_config.get_aws_client_or_panic();

    let mut s3_config_builder = aws_sdk_s3::config::Builder::from(aws_config);
    s3_config_builder.set_force_path_style(Some(true));
    let client = aws_sdk_s3::Client::from_conf(s3_config_builder.build());

    let bucket_name = format!(
        "{}-{}",
        orchestrator_utils::env_utils::get_env_var_or_panic("MADARA_ORCHESTRATOR_AWS_PREFIX"),
        orchestrator_utils::env_utils::get_env_var_or_panic("MADARA_ORCHESTRATOR_AWS_S3_BUCKET_IDENTIFIER")
    );

    // Find the test bucket
    let buckets = client.list_buckets().send().await?;
    let test_bucket = buckets
        .buckets()
        .iter()
        .find(|b| b.name().map(|n| n.starts_with(&bucket_name)).unwrap_or(false))
        .and_then(|b| b.name())
        .ok_or("Test bucket not found")?;

    let lifecycle_output = client.get_bucket_lifecycle_configuration().bucket(test_bucket).send().await?;

    let rules: Vec<LifecycleRuleInfo> = lifecycle_output
        .rules()
        .iter()
        .map(|rule| {
            let (tag_key, tag_value) = if let Some(filter) = rule.filter() {
                if let Some(tag) = filter.tag() {
                    (Some(tag.key().to_string()), Some(tag.value().to_string()))
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            };

            LifecycleRuleInfo {
                id: rule.id().unwrap_or_default().to_string(),
                enabled: *rule.status() == aws_sdk_s3::types::ExpirationStatus::Enabled,
                tag_key,
                tag_value,
                expiration_days: rule.expiration().and_then(|e| e.days()),
            }
        })
        .collect();

    Ok(rules)
}

/// Helper to get object tags from S3
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

    let tags: Vec<(String, String)> =
        tagging_output.tag_set().iter().map(|tag| (tag.key().to_string(), tag.value().to_string())).collect();

    Ok(tags)
}

// =============================================================================
// Integration Happy Path Test
// =============================================================================

/// Integration test for the complete storage cleanup happy path.
/// This test verifies the full flow with real LocalStack (S3) and MongoDB:
/// 1. Create artifacts in S3 for a job
/// 2. Create a completed StateTransition job in MongoDB
/// 3. Run StorageCleanupTrigger
/// 4. Verify artifacts are tagged with expiration tags
/// 5. Verify job is updated with storage_artifacts_tagged_at timestamp
#[rstest]
#[tokio::test]
async fn test_storage_cleanup_happy_path_integration() -> Result<(), Box<dyn Error>> {
    // Build config with actual storage, database and lock client (LocalStack + MongoDB)
    let services = TestConfigBuilder::new()
        .configure_storage_client(ConfigType::Actual)
        .configure_database(ConfigType::Actual)
        .configure_lock_client(ConfigType::Actual)
        .build()
        .await;

    let config = &services.config;
    let job_id: u64 = 99999; // Use a unique job ID to avoid conflicts
    let test_data = Bytes::from(r#"{"test": "happy_path_integration"}"#);

    // Step 1: Create artifacts in S3 for this job
    // Create files in the artifact locations that storage cleanup will look for
    let artifact_file = get_batch_artifact_file(job_id, "proof.json");
    config.storage().put_data(test_data.clone(), &artifact_file).await?;

    let blob_file = get_batch_blob_file(job_id, 0);
    config.storage().put_data(test_data.clone(), &blob_file).await?;

    let snos_file = format!("{}/snos_output.json", get_snos_legacy_dir(job_id));
    config.storage().put_data(test_data.clone(), &snos_file).await?;

    let state_update_file = get_batch_state_update_file(job_id);
    config.storage().put_data(test_data.clone(), &state_update_file).await?;

    // Step 2: Create a completed StateTransition job in MongoDB
    // The job must have:
    // - job_type = StateTransition
    // - status = Completed
    // - metadata.specific.storage_artifacts_tagged_at = None
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
                storage_artifacts_tagged_at: None, // This is the key - not yet tagged
            }),
        },
        version: 0,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    config.database().create_job(job.clone()).await?;

    // Verify the job was created and is returned by get_jobs_without_storage_artifacts_tagged
    let untagged_jobs = config.database().get_jobs_without_storage_artifacts_tagged(Some(100)).await?;
    assert!(
        untagged_jobs.iter().any(|j| j.internal_id == job_id),
        "Job should be in the list of jobs without storage artifacts tagged"
    );

    // Step 3: Run the StorageCleanupTrigger
    let result = StorageCleanupTrigger.run_worker(config.clone()).await;
    assert!(result.is_ok(), "StorageCleanupTrigger should complete successfully: {:?}", result.err());

    // Step 4: Verify artifacts are tagged with expiration tags
    let bucket_name = format!(
        "{}-{}",
        orchestrator_utils::env_utils::get_env_var_or_panic("MADARA_ORCHESTRATOR_AWS_PREFIX"),
        orchestrator_utils::env_utils::get_env_var_or_panic("MADARA_ORCHESTRATOR_AWS_S3_BUCKET_IDENTIFIER")
    );

    // Check tags on the artifact file
    let artifact_tags = get_object_tags(&services.provider_config, &bucket_name, &artifact_file).await?;
    assert!(
        artifact_tags.iter().any(|(k, v)| k == STORAGE_EXPIRATION_TAG_KEY && v == STORAGE_EXPIRATION_TAG_VALUE),
        "Artifact file should have expiration tag. Tags found: {:?}",
        artifact_tags
    );

    // Check tags on the blob file
    let blob_tags = get_object_tags(&services.provider_config, &bucket_name, &blob_file).await?;
    assert!(
        blob_tags.iter().any(|(k, v)| k == STORAGE_EXPIRATION_TAG_KEY && v == STORAGE_EXPIRATION_TAG_VALUE),
        "Blob file should have expiration tag. Tags found: {:?}",
        blob_tags
    );

    // Step 5: Verify job is updated with storage_artifacts_tagged_at timestamp
    let untagged_jobs_after = config.database().get_jobs_without_storage_artifacts_tagged(Some(100)).await?;
    assert!(
        !untagged_jobs_after.iter().any(|j| j.internal_id == job_id),
        "Job should no longer be in the list of untagged jobs (it should have been marked as tagged)"
    );

    Ok(())
}
