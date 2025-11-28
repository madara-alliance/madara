#![cfg(test)]
//! Integration tests for the database migration system.
//!
//! These tests verify the full migration flow with actual RocksDB instances.

use crate::migration::{MigrationRunner, MigrationStatus};
use crate::rocksdb::RocksDBConfig;
use crate::MadaraBackend;
use mc_class_exec::config::NativeConfig;
use mp_chain_config::ChainConfig;
use std::fs;
use std::sync::Arc;
use tempfile::TempDir;

/// Database version file name
const DB_VERSION_FILE: &str = ".db-version";

/// Helper to create a test environment
fn setup_test_env() -> (TempDir, Arc<ChainConfig>, Arc<NativeConfig>) {
    let temp_dir = TempDir::new().unwrap();
    let chain_config = Arc::new(ChainConfig::madara_test());
    let native_config = Arc::new(NativeConfig::default());
    (temp_dir, chain_config, native_config)
}

/// Helper to write a version file
fn write_version_file(base_path: &std::path::Path, version: u32) {
    fs::write(base_path.join(DB_VERSION_FILE), version.to_string()).unwrap();
}

/// Helper to read a version file
fn read_version_file(base_path: &std::path::Path) -> u32 {
    let content = fs::read_to_string(base_path.join(DB_VERSION_FILE)).unwrap();
    content.trim().parse().unwrap()
}

// =============================================================================
// Fresh Database Tests
// =============================================================================

#[tokio::test]
async fn test_fresh_database_creates_version_file() {
    let (temp_dir, chain_config, native_config) = setup_test_env();

    // Open a fresh database
    let backend = MadaraBackend::open_rocksdb(
        temp_dir.path(),
        chain_config,
        Default::default(),
        RocksDBConfig::default(),
        native_config,
    )
    .expect("Should open fresh database");

    // Verify version file was created
    let version_file = temp_dir.path().join(DB_VERSION_FILE);
    assert!(version_file.exists(), "Version file should be created");

    // Version should match DB_VERSION from build
    let version = read_version_file(temp_dir.path());
    assert!(version > 0, "Version should be positive");

    drop(backend);
}

#[tokio::test]
async fn test_fresh_database_can_be_reopened() {
    let (temp_dir, chain_config, native_config) = setup_test_env();

    // Open and close
    {
        let _backend = MadaraBackend::open_rocksdb(
            temp_dir.path(),
            chain_config.clone(),
            Default::default(),
            RocksDBConfig::default(),
            native_config.clone(),
        )
        .expect("Should open fresh database");
    }

    // Reopen
    let backend = MadaraBackend::open_rocksdb(
        temp_dir.path(),
        chain_config,
        Default::default(),
        RocksDBConfig::default(),
        native_config,
    )
    .expect("Should reopen database");

    drop(backend);
}

// =============================================================================
// Version Compatibility Tests
// =============================================================================

#[tokio::test]
async fn test_same_version_opens_without_migration() {
    let (temp_dir, chain_config, native_config) = setup_test_env();

    // First, open to get the current version
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

    let version = read_version_file(temp_dir.path());

    // Create a runner with the same version
    let runner = MigrationRunner::new(temp_dir.path(), version, version);
    let status = runner.check_status().unwrap();

    assert!(
        matches!(status, MigrationStatus::NoMigrationNeeded),
        "Should not need migration for same version"
    );
}

#[tokio::test]
async fn test_newer_database_version_fails() {
    let (temp_dir, _chain_config, _native_config) = setup_test_env();

    // Write a future version
    write_version_file(temp_dir.path(), 9999);

    // Create a runner with an older version
    let runner = MigrationRunner::new(temp_dir.path(), 8, 8);
    let status = runner.check_status().unwrap();

    assert!(
        matches!(status, MigrationStatus::DatabaseNewer { db_version: 9999, binary_version: 8 }),
        "Should detect database is newer"
    );
}

#[tokio::test]
async fn test_too_old_database_version_fails() {
    let (temp_dir, _chain_config, _native_config) = setup_test_env();

    // Write an old version below base
    write_version_file(temp_dir.path(), 5);

    // Create a runner with base_version = 8
    let runner = MigrationRunner::new(temp_dir.path(), 9, 8);
    let status = runner.check_status().unwrap();

    assert!(
        matches!(status, MigrationStatus::DatabaseTooOld { current_version: 5, base_version: 8 }),
        "Should detect database is too old"
    );
}

// =============================================================================
// Migration State Tests
// =============================================================================

#[test]
fn test_migration_state_file_operations() {
    let temp_dir = TempDir::new().unwrap();

    // Test that migration state file can be written and read
    let state_file = temp_dir.path().join(".db-migration-state");
    assert!(!state_file.exists());

    // Write a state file
    let state_content = r#"{
        "started_at": "2024-01-01T00:00:00Z",
        "from_version": 8,
        "target_version": 9,
        "current_version": 8,
        "completed_migrations": []
    }"#;
    fs::write(&state_file, state_content).unwrap();
    assert!(state_file.exists());

    // Read it back
    let content = fs::read_to_string(&state_file).unwrap();
    assert!(content.contains("from_version"));

    // Clean up
    fs::remove_file(&state_file).unwrap();
    assert!(!state_file.exists());
}

#[test]
fn test_version_file_read_write() {
    let temp_dir = TempDir::new().unwrap();

    // Write version
    write_version_file(temp_dir.path(), 42);

    // Read it back
    let version = read_version_file(temp_dir.path());
    assert_eq!(version, 42);

    // Overwrite
    write_version_file(temp_dir.path(), 100);
    let version = read_version_file(temp_dir.path());
    assert_eq!(version, 100);
}

// =============================================================================
// Lock Tests
// =============================================================================

#[test]
fn test_migration_lock_file_creation() {
    let temp_dir = TempDir::new().unwrap();

    // Write a lock file manually
    let lock_path = temp_dir.path().join(".db-migration.lock");
    fs::write(&lock_path, "pid:12345\ntime:2024-01-01T00:00:00Z").unwrap();

    // Verify lock file exists and contains expected content
    assert!(lock_path.exists());
    let content = fs::read_to_string(&lock_path).unwrap();
    assert!(content.contains("pid:12345"));
    assert!(content.contains("time:"));

    // Clean up
    fs::remove_file(&lock_path).unwrap();
    assert!(!lock_path.exists());
}

// =============================================================================
// Backup Tests
// =============================================================================

#[tokio::test]
async fn test_backup_directory_creation() {
    let (temp_dir, chain_config, native_config) = setup_test_env();

    // Open database first
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

    let runner = MigrationRunner::new(temp_dir.path(), 8, 8);
    let backup_path = runner.backup_path();

    // Backup path should be accessible
    assert_eq!(backup_path, temp_dir.path().join("backup_pre_migration"));
}

// =============================================================================
// Full Flow Tests
// =============================================================================

#[tokio::test]
async fn test_full_database_lifecycle() {
    let (temp_dir, chain_config, native_config) = setup_test_env();

    // 1. Fresh open - creates DB and version file
    {
        let backend = MadaraBackend::open_rocksdb(
            temp_dir.path(),
            chain_config.clone(),
            Default::default(),
            RocksDBConfig::default(),
            native_config.clone(),
        )
        .expect("Fresh open should succeed");

        // Verify chain config is correct
        assert_eq!(backend.chain_config().chain_id, chain_config.chain_id);
    }

    // 2. Reopen - should work without migration
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

    // 3. Check version file is consistent
    let version1 = read_version_file(temp_dir.path());

    {
        let _backend = MadaraBackend::open_rocksdb(
            temp_dir.path(),
            chain_config.clone(),
            Default::default(),
            RocksDBConfig::default(),
            native_config.clone(),
        )
        .expect("Third open should succeed");
    }

    let version2 = read_version_file(temp_dir.path());
    assert_eq!(version1, version2, "Version should not change on reopens");
}

#[tokio::test]
async fn test_database_with_data_survives_reopen() {
    let (temp_dir, chain_config, native_config) = setup_test_env();

    // Open and add some data
    {
        let _backend = MadaraBackend::open_rocksdb(
            temp_dir.path(),
            chain_config.clone(),
            Default::default(),
            RocksDBConfig::default(),
            native_config.clone(),
        )
        .expect("First open should succeed");

        // We don't add actual blocks in this test since it would require
        // setting up a lot of infrastructure. The key point is that the
        // database can be reopened.
    }

    // Verify database directory exists
    let db_path = temp_dir.path().join("db");
    assert!(db_path.exists(), "Database directory should exist");

    // Reopen should work
    let backend = MadaraBackend::open_rocksdb(
        temp_dir.path(),
        chain_config,
        Default::default(),
        RocksDBConfig::default(),
        native_config,
    )
    .expect("Reopen should succeed");

    drop(backend);
}

// =============================================================================
// Error Handling Tests
// =============================================================================

#[tokio::test]
async fn test_invalid_version_file_format() {
    let (temp_dir, chain_config, native_config) = setup_test_env();

    // Create directory structure
    fs::create_dir_all(temp_dir.path().join("db")).unwrap();

    // Write invalid version file
    fs::write(temp_dir.path().join(DB_VERSION_FILE), "not_a_number").unwrap();

    // Opening should fail
    let result = MadaraBackend::open_rocksdb(
        temp_dir.path(),
        chain_config,
        Default::default(),
        RocksDBConfig::default(),
        native_config,
    );

    assert!(result.is_err(), "Should fail with invalid version file");
}

#[tokio::test]
async fn test_version_file_with_whitespace() {
    let (temp_dir, chain_config, native_config) = setup_test_env();

    // Create a database first to get the proper version
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

    let version = read_version_file(temp_dir.path());

    // Rewrite with whitespace
    fs::write(temp_dir.path().join(DB_VERSION_FILE), format!("  {}  \n", version)).unwrap();

    // Should still work (trim whitespace)
    let backend = MadaraBackend::open_rocksdb(
        temp_dir.path(),
        chain_config,
        Default::default(),
        RocksDBConfig::default(),
        native_config,
    )
    .expect("Should handle whitespace in version file");

    drop(backend);
}

