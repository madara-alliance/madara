#![cfg(test)]
//! Integration tests for the database migration system.

use crate::migration::{MigrationRunner, MigrationStatus, DB_VERSION_FILE};
use crate::rocksdb::RocksDBConfig;
use crate::MadaraBackend;
use mc_class_exec::config::NativeConfig;
use mp_chain_config::ChainConfig;
use rstest::rstest;
use std::fs;
use std::sync::Arc;
use tempfile::TempDir;

/// Expected DB version from build - update when version changes
const EXPECTED_DB_VERSION: u32 = 9;

fn setup_test_env() -> (TempDir, Arc<ChainConfig>, Arc<NativeConfig>) {
    let temp_dir = TempDir::new().unwrap();
    let chain_config = Arc::new(ChainConfig::madara_test());
    let native_config = Arc::new(NativeConfig::default());
    (temp_dir, chain_config, native_config)
}

// =============================================================================
// Core Tests
// =============================================================================

#[tokio::test]
async fn test_fresh_database_creates_version_file() {
    let (temp_dir, chain_config, native_config) = setup_test_env();

    let backend = MadaraBackend::open_rocksdb(
        temp_dir.path(),
        chain_config,
        Default::default(),
        RocksDBConfig::default(),
        native_config,
    )
    .expect("Should open fresh database");

    // Verify version file created with expected version
    let version_file = temp_dir.path().join(DB_VERSION_FILE);
    assert!(version_file.exists(), "Version file should be created");

    let runner = MigrationRunner::new(temp_dir.path(), EXPECTED_DB_VERSION, EXPECTED_DB_VERSION);
    let version = runner.read_version_file().unwrap();
    assert_eq!(version, EXPECTED_DB_VERSION, "Version should match EXPECTED_DB_VERSION");

    drop(backend);
}

#[tokio::test]
async fn test_full_database_lifecycle() {
    let (temp_dir, chain_config, native_config) = setup_test_env();

    // 1. Fresh open
    {
        let backend = MadaraBackend::open_rocksdb(
            temp_dir.path(),
            chain_config.clone(),
            Default::default(),
            RocksDBConfig::default(),
            native_config.clone(),
        )
        .expect("Fresh open should succeed");
        assert_eq!(backend.chain_config().chain_id, chain_config.chain_id);
    }

    let runner = MigrationRunner::new(temp_dir.path(), EXPECTED_DB_VERSION, EXPECTED_DB_VERSION);
    let version1 = runner.read_version_file().unwrap();

    // 2. Reopen - no migration needed
    {
        let backend = MadaraBackend::open_rocksdb(
            temp_dir.path(),
            chain_config.clone(),
            Default::default(),
            RocksDBConfig::default(),
            native_config.clone(),
        )
        .expect("Reopen should succeed");
        assert_eq!(backend.chain_config().chain_id, chain_config.chain_id);
    }

    // 3. Version unchanged
    let version2 = runner.read_version_file().unwrap();
    assert_eq!(version1, version2, "Version should not change on reopens");
    assert_eq!(version1, EXPECTED_DB_VERSION);
}

#[tokio::test]
async fn test_same_version_opens_without_migration() {
    let (temp_dir, chain_config, native_config) = setup_test_env();

    {
        let _backend = MadaraBackend::open_rocksdb(
            temp_dir.path(),
            chain_config.clone(),
            Default::default(),
            RocksDBConfig::default(),
            native_config.clone(),
        )
        .unwrap();
    }

    let runner = MigrationRunner::new(temp_dir.path(), EXPECTED_DB_VERSION, EXPECTED_DB_VERSION);
    assert!(matches!(runner.check_status().unwrap(), MigrationStatus::NoMigrationNeeded));
}

// =============================================================================
// Version Mismatch Tests (parameterized)
// =============================================================================

#[rstest]
#[case::database_newer(9999, 8, 8, "DatabaseNewer")]
#[case::database_too_old(5, 9, 8, "DatabaseTooOld")]
fn test_version_mismatch(
    #[case] db_version: u32,
    #[case] required_version: u32,
    #[case] base_version: u32,
    #[case] expected_status: &str,
) {
    let temp_dir = TempDir::new().unwrap();

    // Write the version file
    let runner = MigrationRunner::new(temp_dir.path(), required_version, base_version);
    fs::write(temp_dir.path().join(DB_VERSION_FILE), db_version.to_string()).unwrap();

    let status = runner.check_status().unwrap();
    let status_name = format!("{:?}", status);
    assert!(status_name.starts_with(expected_status), "Expected {} but got {}", expected_status, status_name);
}

// =============================================================================
// Error Handling Tests
// =============================================================================

#[rstest]
#[case::invalid_format("not_a_number", true)]
#[case::whitespace_ok("  8  \n", false)]
#[case::empty("", true)]
fn test_version_file_parsing(#[case] content: &str, #[case] should_fail: bool) {
    let temp_dir = TempDir::new().unwrap();
    fs::write(temp_dir.path().join(DB_VERSION_FILE), content).unwrap();

    let runner = MigrationRunner::new(temp_dir.path(), EXPECTED_DB_VERSION, EXPECTED_DB_VERSION);
    let result = runner.read_version_file();

    if should_fail {
        assert!(result.is_err(), "Should fail for content: {:?}", content);
    } else {
        assert!(result.is_ok(), "Should succeed for content: {:?}", content);
    }
}

#[tokio::test]
async fn test_invalid_version_file_blocks_open() {
    let (temp_dir, chain_config, native_config) = setup_test_env();

    fs::create_dir_all(temp_dir.path().join("db")).unwrap();
    fs::write(temp_dir.path().join(DB_VERSION_FILE), "invalid").unwrap();

    let result = MadaraBackend::open_rocksdb(
        temp_dir.path(),
        chain_config,
        Default::default(),
        RocksDBConfig::default(),
        native_config,
    );

    assert!(result.is_err(), "Should fail with invalid version file");
}

// =============================================================================
// Backup Path Test
// =============================================================================

#[test]
fn test_backup_path() {
    let temp_dir = TempDir::new().unwrap();
    let runner = MigrationRunner::new(temp_dir.path(), 8, 8);
    assert_eq!(runner.backup_path(), temp_dir.path().join("backup_pre_migration"));
}
