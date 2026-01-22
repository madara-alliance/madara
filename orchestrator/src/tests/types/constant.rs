// Tests for path helper functions in types/constant.rs

use crate::types::constant::{
    get_batch_artifact_file, get_batch_artifacts_dir, get_batch_blob_dir, get_batch_blob_file,
    get_batch_state_update_file, get_snos_legacy_dir,
};

#[test]
fn test_path_helpers_format_and_structure() {
    let job_id: u64 = 12345;

    // Artifacts
    assert_eq!(get_batch_artifacts_dir(job_id), "artifacts/batch/12345");
    assert_eq!(get_batch_artifact_file(job_id, "proof.json"), "artifacts/batch/12345/proof.json");
    assert_eq!(get_batch_artifact_file(job_id, "nested/file.txt"), "artifacts/batch/12345/nested/file.txt");

    // Blobs
    assert_eq!(get_batch_blob_dir(job_id), "blob/batch/12345");
    assert_eq!(get_batch_blob_file(job_id, 0), "blob/batch/12345/0.txt");
    assert_eq!(get_batch_blob_file(job_id, 99), "blob/batch/12345/99.txt");

    // State update
    assert_eq!(get_batch_state_update_file(job_id), "state_update/batch/12345.json");

    // SNOS legacy (root level, no path separators)
    let snos_dir = get_snos_legacy_dir(job_id);
    assert_eq!(snos_dir, "12345");
    assert!(!snos_dir.contains('/'), "SNOS legacy dir should be at root level");
}

#[test]
fn test_path_helpers_edge_cases() {
    // Zero
    assert_eq!(get_batch_artifacts_dir(0), "artifacts/batch/0");
    assert_eq!(get_batch_blob_dir(0), "blob/batch/0");
    assert_eq!(get_snos_legacy_dir(0), "0");
    assert_eq!(get_batch_state_update_file(0), "state_update/batch/0.json");

    // Large value
    let large_id: u64 = u64::MAX;
    assert!(get_batch_artifacts_dir(large_id).contains(&large_id.to_string()));
}
