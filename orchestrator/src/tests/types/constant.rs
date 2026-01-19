// Tests for path helper functions in types/constant.rs
//
// These functions are used by multiple parts of the codebase (storage cleanup,
// job handlers, etc.) so they are tested separately for better visibility and
// maintainability.

use crate::types::constant::{
    get_batch_artifact_file, get_batch_artifacts_dir, get_batch_blob_dir, get_batch_blob_file,
    get_batch_state_update_file, get_snos_legacy_dir,
};

/// Test that all path helper functions return correctly formatted paths.
/// This is critical because wrong paths mean wrong files get tagged/read/written.
#[test]
fn test_path_helpers_correct_format() {
    let job_id: u64 = 12345;

    // Test artifacts directory path
    let artifacts_dir = get_batch_artifacts_dir(job_id);
    assert_eq!(artifacts_dir, "artifacts/batch/12345");

    // Test artifact file path
    let artifact_file = get_batch_artifact_file(job_id, "proof.json");
    assert_eq!(artifact_file, "artifacts/batch/12345/proof.json");

    // Test blob directory path
    let blob_dir = get_batch_blob_dir(job_id);
    assert_eq!(blob_dir, "blob/batch/12345");

    // Test blob file path
    let blob_file = get_batch_blob_file(job_id, 0);
    assert_eq!(blob_file, "blob/batch/12345/0.txt");

    // Test state update file path
    let state_update_file = get_batch_state_update_file(job_id);
    assert_eq!(state_update_file, "state_update/batch/12345.json");
}

/// Test SNOS legacy directory path format (at root level for backward compatibility)
#[test]
fn test_get_snos_legacy_dir_root_level() {
    let job_id: u64 = 99999;
    let snos_dir = get_snos_legacy_dir(job_id);

    // SNOS legacy dir should be at root level (just the job_id)
    assert_eq!(snos_dir, "99999");

    // Verify it doesn't have any path separators (confirming root level)
    assert!(!snos_dir.contains('/'), "SNOS legacy dir should be at root level without path separators");
}

/// Test path helpers with edge cases - zero and large values
#[test]
fn test_path_helpers_edge_cases() {
    // Test with zero
    assert_eq!(get_batch_artifacts_dir(0), "artifacts/batch/0");
    assert_eq!(get_batch_blob_dir(0), "blob/batch/0");
    assert_eq!(get_snos_legacy_dir(0), "0");
    assert_eq!(get_batch_state_update_file(0), "state_update/batch/0.json");

    // Test with large values
    let large_id: u64 = u64::MAX;
    let artifacts_dir = get_batch_artifacts_dir(large_id);
    assert!(artifacts_dir.starts_with("artifacts/batch/"));
    assert!(artifacts_dir.contains(&large_id.to_string()));
}

/// Test that blob file paths use correct index formatting
#[test]
fn test_blob_file_index_formatting() {
    let job_id: u64 = 100;

    // Test various blob indices
    assert_eq!(get_batch_blob_file(job_id, 0), "blob/batch/100/0.txt");
    assert_eq!(get_batch_blob_file(job_id, 1), "blob/batch/100/1.txt");
    assert_eq!(get_batch_blob_file(job_id, 99), "blob/batch/100/99.txt");
}

/// Test artifact file paths with different file names
#[test]
fn test_artifact_file_with_different_names() {
    let job_id: u64 = 42;

    assert_eq!(get_batch_artifact_file(job_id, "proof.json"), "artifacts/batch/42/proof.json");
    assert_eq!(get_batch_artifact_file(job_id, "output.bin"), "artifacts/batch/42/output.bin");
    assert_eq!(get_batch_artifact_file(job_id, "nested/file.txt"), "artifacts/batch/42/nested/file.txt");
}
