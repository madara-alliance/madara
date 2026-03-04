// Storage Cleanup Worker Tests
// Unit tests with mocks for orchestration logic + Integration test with LocalStack/MongoDB

use bytes::Bytes;
use rstest::rstest;
use std::collections::BTreeSet;
use std::error::Error;
use std::sync::{Arc, Mutex};

use crate::core::client::database::{DatabaseError, MockDatabaseClient};
use crate::core::client::lock::error::LockError;
use crate::core::client::lock::{LockResult, MockLockClient};
use crate::core::client::storage::MockStorageClient;
use crate::tests::config::{ConfigType, TestConfigBuilder};
use crate::tests::utils::build_snos_batch;
use crate::tests::workers::utils::get_job_by_mock_id_vector;
use crate::types::constant::{
    get_batch_artifact_file, get_batch_blob_dir, get_batch_blob_file, get_batch_state_update_file, get_snos_batch_dir,
    BLOB_DATA_FILE_NAME, CAIRO_PIE_FILE_NAME, DA_SEGMENT_FILE_NAME, MAX_BLOBS, PROGRAM_OUTPUT_FILE_NAME,
    PROOF_FILE_NAME, PROOF_PART2_FILE_NAME, SNOS_OUTPUT_FILE_NAME, STORAGE_EXPIRATION_TAG_KEY,
    STORAGE_EXPIRATION_TAG_VALUE,
};
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{
    CommonMetadata, JobMetadata, JobSpecificMetadata, SettlementContext, SettlementContextData, StateUpdateMetadata,
};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::types::Layer;
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
    database.expect_get_snos_batches_by_aggregator_index().times(0);
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

/// Test: DB failure while fetching SNOS batches should skip tagging and not mark job as tagged
#[rstest]
#[tokio::test]
async fn test_process_completed_jobs_skips_on_snos_batch_lookup_failure() -> Result<(), Box<dyn Error>> {
    let mut database = MockDatabaseClient::new();
    let mut storage = MockStorageClient::new();
    let mut lock = MockLockClient::new();

    lock.expect_acquire_lock().returning(|_, _, _, _| Ok(LockResult::Acquired));
    lock.expect_release_lock().returning(|_, _| Ok(LockResult::Released));

    let job_id: u64 = 42;
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

    database.expect_get_jobs_without_storage_artifacts_tagged().returning(move |_| Ok(vec![job.clone()]));
    database
        .expect_get_snos_batches_by_aggregator_index()
        .returning(|_| Err(DatabaseError::KeyNotFound("db error".to_string())));

    storage.expect_tag_object().never();
    database.expect_update_job().never();

    let services = TestConfigBuilder::new()
        .configure_storage_client(storage.into())
        .configure_database(database.into())
        .configure_lock_client(lock.into())
        .build()
        .await;

    let result = StorageCleanupTrigger.run_worker(services.config).await;
    assert!(result.is_ok(), "Worker should continue even when SNOS batch lookup fails");
    Ok(())
}

/// Test: L2 uses direct SNOS file paths (3 files) and never lists directories
#[rstest]
#[tokio::test]
async fn test_storage_cleanup_l2_direct_snos_paths_without_listing() -> Result<(), Box<dyn Error>> {
    let mut database = MockDatabaseClient::new();
    let mut storage = MockStorageClient::new();
    let mut lock = MockLockClient::new();

    lock.expect_acquire_lock().returning(|_, _, _, _| Ok(LockResult::Acquired));
    lock.expect_release_lock().returning(|_, _| Ok(LockResult::Released));

    let job_id: u64 = 4242;
    let snos_batch_ids = vec![10001_u64, 10002_u64];

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
                blob_data_path: None,
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

    let snos_batches = snos_batch_ids
        .iter()
        .enumerate()
        .map(|(idx, snos_batch_id)| build_snos_batch(*snos_batch_id, Some(job_id), 1000 + idx as u64))
        .collect::<Vec<_>>();

    database.expect_get_jobs_without_storage_artifacts_tagged().returning(move |_| Ok(vec![job.clone()]));
    database.expect_get_snos_batches_by_aggregator_index().returning(move |_| Ok(snos_batches.clone()));
    database.expect_update_job().returning(|_, _| Ok(()));

    let mut expected = BTreeSet::new();
    for file in
        [CAIRO_PIE_FILE_NAME, DA_SEGMENT_FILE_NAME, PROGRAM_OUTPUT_FILE_NAME, SNOS_OUTPUT_FILE_NAME, PROOF_FILE_NAME]
    {
        expected.insert(get_batch_artifact_file(job_id, file));
    }
    for blob_index in 0..=MAX_BLOBS {
        expected.insert(get_batch_blob_file(job_id, blob_index as u64));
    }
    expected.insert(get_batch_state_update_file(job_id));
    for snos_batch_id in &snos_batch_ids {
        let snos_dir = get_snos_batch_dir(*snos_batch_id);
        for file in [CAIRO_PIE_FILE_NAME, SNOS_OUTPUT_FILE_NAME, PROGRAM_OUTPUT_FILE_NAME] {
            expected.insert(format!("{}/{}", snos_dir, file));
        }
    }

    let expected = Arc::new(expected);
    let seen = Arc::new(Mutex::new(BTreeSet::new()));
    let expected_len = expected.len();

    storage.expect_tag_object().times(expected_len).returning({
        let expected = Arc::clone(&expected);
        let seen = Arc::clone(&seen);
        move |key, _| {
            assert!(expected.contains(key), "Unexpected key tagged: {}", key);
            seen.lock().unwrap().insert(key.to_string());
            Ok(())
        }
    });

    let services = TestConfigBuilder::new()
        .configure_storage_client(storage.into())
        .configure_database(database.into())
        .configure_lock_client(lock.into())
        .build()
        .await;

    let result = StorageCleanupTrigger.run_worker(services.config).await;
    assert!(result.is_ok(), "Worker should complete");

    let seen = seen.lock().unwrap();
    assert_eq!(*seen, *expected, "Tagged keys should match expected set");

    Ok(())
}

/// Test: L3 uses direct SNOS file paths (6 files) and never lists directories
#[rstest]
#[tokio::test]
async fn test_storage_cleanup_l3_direct_snos_paths_without_listing() -> Result<(), Box<dyn Error>> {
    let mut database = MockDatabaseClient::new();
    let mut storage = MockStorageClient::new();
    let mut lock = MockLockClient::new();

    lock.expect_acquire_lock().returning(|_, _, _, _| Ok(LockResult::Acquired));
    lock.expect_release_lock().returning(|_, _| Ok(LockResult::Released));

    let job_id: u64 = 7777;

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
                blob_data_path: None,
                da_segment_path: None,
                tx_hash: Some("0xdef".to_string()),
                context: SettlementContext::Block(SettlementContextData { to_settle: job_id, last_failed: None }),
                storage_artifacts_tagged_at: None,
            }),
        },
        version: 0,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    database.expect_get_jobs_without_storage_artifacts_tagged().returning(move |_| Ok(vec![job.clone()]));
    database.expect_get_snos_batches_by_aggregator_index().times(0);
    database.expect_update_job().returning(|_, _| Ok(()));

    let mut expected = BTreeSet::new();
    for file in
        [CAIRO_PIE_FILE_NAME, DA_SEGMENT_FILE_NAME, PROGRAM_OUTPUT_FILE_NAME, SNOS_OUTPUT_FILE_NAME, PROOF_FILE_NAME]
    {
        expected.insert(get_batch_artifact_file(job_id, file));
    }
    for blob_index in 0..=MAX_BLOBS {
        expected.insert(get_batch_blob_file(job_id, blob_index as u64));
    }
    expected.insert(get_batch_state_update_file(job_id));

    let snos_dir = get_snos_batch_dir(job_id);
    for file in [
        CAIRO_PIE_FILE_NAME,
        SNOS_OUTPUT_FILE_NAME,
        PROGRAM_OUTPUT_FILE_NAME,
        BLOB_DATA_FILE_NAME,
        PROOF_FILE_NAME,
        PROOF_PART2_FILE_NAME,
    ] {
        expected.insert(format!("{}/{}", snos_dir, file));
    }

    let expected = Arc::new(expected);
    let seen = Arc::new(Mutex::new(BTreeSet::new()));
    let expected_len = expected.len();

    storage.expect_tag_object().times(expected_len).returning({
        let expected = Arc::clone(&expected);
        let seen = Arc::clone(&seen);
        move |key, _| {
            assert!(expected.contains(key), "Unexpected key tagged: {}", key);
            seen.lock().unwrap().insert(key.to_string());
            Ok(())
        }
    });

    let services = TestConfigBuilder::new()
        .configure_storage_client(storage.into())
        .configure_database(database.into())
        .configure_lock_client(lock.into())
        .build()
        .await;

    let result = StorageCleanupTrigger.run_worker(services.config).await;
    assert!(result.is_ok(), "Worker should complete");

    let seen = seen.lock().unwrap();
    assert_eq!(*seen, *expected, "Tagged keys should match expected set");

    Ok(())
}

// =============================================================================
// Integration Tests with LocalStack + MongoDB
// =============================================================================

/// Full integration test (L2): Creates L2 artifacts + MongoDB job, runs worker, verifies tagging
#[rstest]
#[tokio::test]
async fn test_storage_cleanup_happy_path_integration_l2() -> Result<(), Box<dyn Error>> {
    let services = TestConfigBuilder::new()
        .configure_storage_client(ConfigType::Actual)
        .configure_database(ConfigType::Actual)
        .configure_lock_client(ConfigType::Actual)
        .configure_layer(Layer::L2)
        .build()
        .await;

    let config = &services.config;
    let agg_batch_id: u64 = 2815;
    let snos_batch_ids = vec![10000_u64, 10001_u64];
    let blob_index: u64 = 1;
    let test_data = Bytes::from(r#"{"test": "happy_path_integration_l2"}"#);

    // Step 1: Create artifacts in S3 (aggregator + SNOS batches)
    let mut keys: Vec<String> = Vec::new();
    let artifact_files = vec![
        get_batch_artifact_file(agg_batch_id, CAIRO_PIE_FILE_NAME),
        get_batch_artifact_file(agg_batch_id, DA_SEGMENT_FILE_NAME),
        get_batch_artifact_file(agg_batch_id, PROGRAM_OUTPUT_FILE_NAME),
        get_batch_artifact_file(agg_batch_id, PROOF_FILE_NAME),
    ];
    keys.extend(artifact_files);

    let blob_file = get_batch_blob_file(agg_batch_id, blob_index);
    keys.push(blob_file);

    let state_update_file = get_batch_state_update_file(agg_batch_id);
    keys.push(state_update_file);

    for snos_batch_id in &snos_batch_ids {
        keys.extend(snos_batch_files(*snos_batch_id));
    }

    for key in &keys {
        config.storage().put_data(test_data.clone(), key).await?;
    }

    // Step 1b: Create SNOS batches linked to this aggregator batch
    for (idx, snos_batch_id) in snos_batch_ids.iter().enumerate() {
        let start_block = 1000 + (idx as u64 * 100);
        let snos_batch = build_snos_batch(*snos_batch_id, Some(agg_batch_id), start_block);
        config.database().create_snos_batch(snos_batch).await?;
    }

    // Step 2: Create completed StateTransition job in MongoDB (L2 batch settlement)
    let job = JobItem {
        id: uuid::Uuid::new_v4(),
        internal_id: agg_batch_id,
        job_type: JobType::StateTransition,
        status: JobStatus::Completed,
        external_id: crate::types::jobs::external_id::ExternalId::Number(agg_batch_id as usize),
        metadata: JobMetadata {
            common: CommonMetadata::default(),
            specific: JobSpecificMetadata::StateUpdate(StateUpdateMetadata {
                snos_output_path: Some(get_batch_artifact_file(agg_batch_id, SNOS_OUTPUT_FILE_NAME)),
                program_output_path: Some(get_batch_artifact_file(agg_batch_id, PROGRAM_OUTPUT_FILE_NAME)),
                blob_data_path: Some(get_batch_blob_dir(agg_batch_id)),
                da_segment_path: Some(get_batch_artifact_file(agg_batch_id, DA_SEGMENT_FILE_NAME)),
                tx_hash: Some("0x123".to_string()),
                context: SettlementContext::Batch(SettlementContextData { to_settle: agg_batch_id, last_failed: None }),
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
    assert!(untagged_jobs.iter().any(|j| j.internal_id == agg_batch_id), "Job should be in untagged list");

    // Step 3: Run StorageCleanupTrigger
    let result = StorageCleanupTrigger.run_worker(config.clone()).await;
    assert!(result.is_ok(), "StorageCleanupTrigger should complete: {:?}", result.err());

    // Step 4: Verify artifacts are tagged
    let bucket_name = services.storage_params.bucket_identifier.to_string();
    for key in &keys {
        assert_expiration_tagged(&services.provider_config, &bucket_name, key).await?;
    }

    // Step 5: Verify job is no longer in untagged list
    let untagged_jobs_after = config.database().get_jobs_without_storage_artifacts_tagged(Some(100)).await?;
    assert!(!untagged_jobs_after.iter().any(|j| j.internal_id == agg_batch_id), "Job should be marked as tagged");

    Ok(())
}

/// Full integration test (L3): Creates L3 artifacts + MongoDB job, runs worker, verifies tagging
#[rstest]
#[tokio::test]
async fn test_storage_cleanup_happy_path_integration_l3() -> Result<(), Box<dyn Error>> {
    let services = TestConfigBuilder::new()
        .configure_storage_client(ConfigType::Actual)
        .configure_database(ConfigType::Actual)
        .configure_lock_client(ConfigType::Actual)
        .configure_layer(Layer::L3)
        .build()
        .await;

    let config = &services.config;
    let block_no: u64 = 12345;
    let test_data = Bytes::from(r#"{"test": "happy_path_integration_l3"}"#);

    // Step 1: Create artifacts in S3 (root-level for the block)
    let mut keys = l3_block_files(block_no);
    let state_update_file = get_batch_state_update_file(block_no);
    keys.push(state_update_file);

    for key in &keys {
        config.storage().put_data(test_data.clone(), key).await?;
    }

    // Step 2: Create completed StateTransition job in MongoDB (L3 block settlement)
    let job = JobItem {
        id: uuid::Uuid::new_v4(),
        internal_id: block_no,
        job_type: JobType::StateTransition,
        status: JobStatus::Completed,
        external_id: crate::types::jobs::external_id::ExternalId::Number(block_no as usize),
        metadata: JobMetadata {
            common: CommonMetadata::default(),
            specific: JobSpecificMetadata::StateUpdate(StateUpdateMetadata {
                snos_output_path: Some(format!("{}/{}", get_snos_batch_dir(block_no), SNOS_OUTPUT_FILE_NAME)),
                program_output_path: Some(format!("{}/{}", get_snos_batch_dir(block_no), PROGRAM_OUTPUT_FILE_NAME)),
                blob_data_path: Some(format!("{}/{}", get_snos_batch_dir(block_no), BLOB_DATA_FILE_NAME)),
                da_segment_path: None,
                tx_hash: Some("0x456".to_string()),
                context: SettlementContext::Block(SettlementContextData { to_settle: block_no, last_failed: None }),
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
    assert!(untagged_jobs.iter().any(|j| j.internal_id == block_no), "Job should be in untagged list");

    // Step 3: Run StorageCleanupTrigger
    let result = StorageCleanupTrigger.run_worker(config.clone()).await;
    assert!(result.is_ok(), "StorageCleanupTrigger should complete: {:?}", result.err());

    // Step 4: Verify artifacts are tagged
    let bucket_name = services.storage_params.bucket_identifier.to_string();
    for key in &keys {
        assert_expiration_tagged(&services.provider_config, &bucket_name, key).await?;
    }

    // Step 5: Verify job is no longer in untagged list
    let untagged_jobs_after = config.database().get_jobs_without_storage_artifacts_tagged(Some(100)).await?;
    assert!(!untagged_jobs_after.iter().any(|j| j.internal_id == block_no), "Job should be marked as tagged");

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
    let tags = [(STORAGE_EXPIRATION_TAG_KEY, STORAGE_EXPIRATION_TAG_VALUE)];
    storage.tag_object(test_key, &tags).await?;

    let bucket_name = services.storage_params.bucket_identifier.to_string();
    let applied_tags = get_object_tags(&services.provider_config, &bucket_name, test_key).await?;
    assert!(
        applied_tags.iter().any(|(k, v)| k == STORAGE_EXPIRATION_TAG_KEY && v == STORAGE_EXPIRATION_TAG_VALUE),
        "Tag should be applied: {:?}",
        applied_tags
    );

    // Test 2: tag_nonexistent_object returns NotFound
    let result = storage.tag_object("nonexistent/path/file.json", &tags).await;
    assert!(
        matches!(result, Err(crate::core::client::storage::StorageError::NotFound(_))),
        "Expected NotFound for nonexistent object, got: {:?}",
        result
    );

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

fn snos_batch_files(batch_id: u64) -> Vec<String> {
    let dir = get_snos_batch_dir(batch_id);
    vec![
        format!("{}/{}", dir, CAIRO_PIE_FILE_NAME),
        format!("{}/{}", dir, SNOS_OUTPUT_FILE_NAME),
        format!("{}/{}", dir, PROGRAM_OUTPUT_FILE_NAME),
    ]
}

fn l3_block_files(block_no: u64) -> Vec<String> {
    let mut files = snos_batch_files(block_no);
    files.extend([
        format!("{}/{}", block_no, BLOB_DATA_FILE_NAME),
        format!("{}/{}", block_no, PROOF_FILE_NAME),
        format!("{}/{}", block_no, PROOF_PART2_FILE_NAME),
    ]);
    files
}

async fn assert_expiration_tagged(
    provider_config: &Arc<crate::core::cloud::CloudProvider>,
    bucket_name: &str,
    key: &str,
) -> Result<(), Box<dyn Error>> {
    let tags = get_object_tags(provider_config, bucket_name, key).await?;
    assert!(
        tags.iter().any(|(k, v)| k == STORAGE_EXPIRATION_TAG_KEY && v == STORAGE_EXPIRATION_TAG_VALUE),
        "File {} should have expiration tag: {:?}",
        key,
        tags
    );
    Ok(())
}

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
