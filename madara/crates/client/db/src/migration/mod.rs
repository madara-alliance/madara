//! Database migration system for Madara.
//!
//! This module provides a migration framework for upgrading the database schema
//! between versions. It handles:
//!
//! - Version detection and comparison
//! - Sequential migration execution
//! - Crash recovery via state checkpointing
//! - Pre-migration backups
//! - Progress reporting for long-running migrations
//!
//! # Architecture
//!
//! The migration system is inspired by Pathfinder's approach but adapted for RocksDB.
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                      MigrationRunner                             │
//! │  - Checks current vs required version                           │
//! │  - Determines which migrations to run                           │
//! │  - Executes migrations sequentially                             │
//! │  - Handles crash recovery                                       │
//! └─────────────────────────────────────────────────────────────────┘
//!                              │
//!                              ▼
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                      Migration Registry                          │
//! │  - Maps version numbers to migration functions                  │
//! │  - Validates migration chain                                    │
//! └─────────────────────────────────────────────────────────────────┘
//!                              │
//!                              ▼
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                      Individual Migrations                       │
//! │  - revision_0009.rs: v8 -> v9 (example)                         │
//! │  - revision_0010.rs: v9 -> v10                                  │
//! │  - ...                                                          │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! The migration system is automatically invoked when opening the database:
//!
//! ```ignore
//! let backend = MadaraBackend::open_rocksdb(
//!     path,
//!     chain_config,
//!     backend_config,
//!     rocksdb_config,
//!     native_config,
//! )?;
//! // Migrations run automatically if needed
//! ```
//!
//! # Files
//!
//! The migration system uses several files in the database directory:
//!
//! - `.db-version`: Contains the current database version (single integer)
//! - `.db-migration.lock`: Lock file to prevent concurrent migrations
//! - `.db-migration-state`: Checkpoint file for crash recovery
//!
//! # Adding a New Migration
//!
//! See the documentation in [`revisions`] module for instructions.

mod context;
mod error;
mod registry;
pub mod revisions;

pub use context::{MigrationContext, MigrationProgress, ProgressCallback};
pub use error::MigrationError;
pub use registry::{get_migrations, get_migrations_for_range, validate_registry, Migration, MigrationFn};

use rocksdb::DB;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// File name for database version.
const DB_VERSION_FILE: &str = ".db-version";

/// File name for migration lock.
const DB_MIGRATION_LOCK: &str = ".db-migration.lock";

/// File name for migration state (crash recovery).
const DB_MIGRATION_STATE: &str = ".db-migration-state";

/// Directory name for pre-migration backup.
const BACKUP_DIR_NAME: &str = "backup_pre_migration";

/// Result of checking migration status.
#[derive(Debug)]
pub enum MigrationStatus {
    /// Fresh database - will be created at current version.
    FreshDatabase,

    /// Database is already at the required version.
    NoMigrationNeeded,

    /// Migrations need to be applied.
    MigrationRequired {
        /// Current database version
        current_version: u32,
        /// Target version
        target_version: u32,
        /// Number of migrations to apply
        migration_count: usize,
    },

    /// Database version is older than the minimum supported.
    DatabaseTooOld {
        /// Current database version
        current_version: u32,
        /// Minimum supported version
        base_version: u32,
    },

    /// Database version is newer than the binary supports.
    DatabaseNewer {
        /// Database version
        db_version: u32,
        /// Binary's expected version
        binary_version: u32,
    },
}

/// Saved migration state for crash recovery.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct MigrationState {
    /// When the migration started
    started_at: String,
    /// Source version
    from_version: u32,
    /// Target version
    target_version: u32,
    /// Version we're currently at (last successfully completed migration)
    current_version: u32,
    /// List of completed migration target versions
    completed_migrations: Vec<u32>,
}

/// Main migration orchestrator.
///
/// Handles checking migration status, executing migrations, and crash recovery.
pub struct MigrationRunner {
    /// Base path to the database directory
    base_path: PathBuf,
    /// Version required by the binary
    required_version: u32,
    /// Minimum version that can be migrated from
    base_version: u32,
    /// Flag for graceful abort
    abort_flag: Arc<AtomicBool>,
}

impl MigrationRunner {
    /// Create a new migration runner.
    ///
    /// # Arguments
    ///
    /// * `base_path` - Path to the database directory
    /// * `required_version` - Version required by the binary
    /// * `base_version` - Minimum version that can be migrated from
    pub fn new(base_path: &Path, required_version: u32, base_version: u32) -> Self {
        Self {
            base_path: base_path.to_path_buf(),
            required_version,
            base_version,
            abort_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Set the abort flag to signal migration should stop.
    pub fn abort(&self) {
        self.abort_flag.store(true, Ordering::Relaxed);
    }

    /// Check migration status without running anything.
    ///
    /// Returns information about what migrations (if any) need to be applied.
    pub fn check_status(&self) -> Result<MigrationStatus, MigrationError> {
        let version_file = self.base_path.join(DB_VERSION_FILE);

        // Fresh database
        if !version_file.exists() {
            return Ok(MigrationStatus::FreshDatabase);
        }

        let current_version = self.read_version_file()?;

        // Already at target version
        if current_version == self.required_version {
            return Ok(MigrationStatus::NoMigrationNeeded);
        }

        // Database is newer than binary
        if current_version > self.required_version {
            return Ok(MigrationStatus::DatabaseNewer {
                db_version: current_version,
                binary_version: self.required_version,
            });
        }

        // Database is too old
        if current_version < self.base_version {
            return Ok(MigrationStatus::DatabaseTooOld { current_version, base_version: self.base_version });
        }

        // Check if migration path exists
        let migrations = get_migrations_for_range(current_version, self.required_version)?;

        Ok(MigrationStatus::MigrationRequired {
            current_version,
            target_version: self.required_version,
            migration_count: migrations.len(),
        })
    }

    /// Run migrations if needed.
    ///
    /// This is the main entry point for the migration system.
    /// It will:
    /// 1. Check if migrations are needed
    /// 2. Create a backup
    /// 3. Run migrations sequentially
    /// 4. Update version file after each migration
    /// 5. Clean up on success
    ///
    /// # Arguments
    ///
    /// * `db` - Open RocksDB instance
    ///
    /// # Returns
    ///
    /// Ok(()) if no migrations needed or all migrations completed successfully.
    pub fn run_migrations(&self, db: &DB) -> Result<(), MigrationError> {
        let status = self.check_status()?;

        match status {
            MigrationStatus::FreshDatabase => {
                tracing::info!("📦 Fresh database, creating at version {}", self.required_version);
                self.write_version_file(self.required_version)?;
                Ok(())
            }
            MigrationStatus::NoMigrationNeeded => {
                tracing::debug!("✅ Database version {} matches binary, no migration needed", self.required_version);
                Ok(())
            }
            MigrationStatus::DatabaseNewer { db_version, binary_version } => {
                Err(MigrationError::DatabaseNewerThanBinary { db_version, binary_version })
            }
            MigrationStatus::DatabaseTooOld { current_version, base_version } => {
                Err(MigrationError::DatabaseTooOld { current_version, base_version })
            }
            MigrationStatus::MigrationRequired { current_version, target_version, migration_count } => {
                self.execute_migrations(db, current_version, target_version, migration_count)
            }
        }
    }

    /// Execute the actual migrations.
    fn execute_migrations(
        &self,
        db: &DB,
        from_version: u32,
        to_version: u32,
        migration_count: usize,
    ) -> Result<(), MigrationError> {
        // Check for interrupted migration
        if let Some(state) = self.load_migration_state()? {
            tracing::warn!(
                "⚠️  Found interrupted migration from v{} to v{}, resuming from v{}...",
                state.from_version,
                state.target_version,
                state.current_version
            );
            return self.resume_migration(db, state);
        }

        // Acquire migration lock
        let _lock = self.acquire_lock()?;

        tracing::info!(
            "🔄 Starting database migration from v{} to v{} ({} migration(s))",
            from_version,
            to_version,
            migration_count
        );

        // Get migrations
        let migrations = get_migrations_for_range(from_version, to_version)?;

        // Save initial state for crash recovery
        let mut state = MigrationState {
            started_at: chrono::Utc::now().to_rfc3339(),
            from_version,
            target_version: to_version,
            current_version: from_version,
            completed_migrations: vec![],
        };
        self.save_migration_state(&state)?;

        // Create backup before migration
        tracing::info!("📸 Creating pre-migration backup...");
        self.create_backup(db)?;

        // Execute migrations one by one
        for migration in migrations {
            if self.abort_flag.load(Ordering::Relaxed) {
                tracing::warn!("⚠️  Migration aborted by user");
                return Err(MigrationError::Aborted);
            }

            tracing::info!(
                "📦 Running migration '{}' (v{} -> v{})",
                migration.name,
                migration.from_version,
                migration.to_version
            );

            let context = MigrationContext::new(db, migration.from_version, migration.to_version, self.abort_flag.clone())
                .with_progress_callback(Box::new(|progress| {
                    if progress.total_steps > 0 {
                        tracing::info!(
                            "   Progress: [{}/{}] {}",
                            progress.current_step,
                            progress.total_steps,
                            progress.message
                        );
                    } else {
                        tracing::info!("   Progress: [{}] {}", progress.current_step, progress.message);
                    }
                }));

            let start_time = std::time::Instant::now();

            match (migration.migrate)(&context) {
                Ok(()) => {
                    let elapsed = start_time.elapsed();
                    tracing::info!("✅ Migration '{}' completed in {:.2}s", migration.name, elapsed.as_secs_f64());

                    // Update state
                    state.current_version = migration.to_version;
                    state.completed_migrations.push(migration.to_version);
                    self.save_migration_state(&state)?;

                    // Update version file after each successful migration
                    self.write_version_file(migration.to_version)?;
                }
                Err(e) => {
                    tracing::error!("❌ Migration '{}' failed: {}", migration.name, e);
                    tracing::error!(
                        "Database may be in inconsistent state. \
                        You can restore from backup at: {:?}",
                        self.base_path.join(BACKUP_DIR_NAME)
                    );
                    return Err(MigrationError::MigrationStepFailed {
                        name: migration.name.to_string(),
                        from_version: migration.from_version,
                        to_version: migration.to_version,
                        message: e.to_string(),
                    });
                }
            }
        }

        // Clean up on success
        self.cleanup_migration_state()?;

        tracing::info!("🎉 Database migration completed successfully! Now at version {}", to_version);

        Ok(())
    }

    /// Resume an interrupted migration.
    fn resume_migration(&self, db: &DB, state: MigrationState) -> Result<(), MigrationError> {
        let _lock = self.acquire_lock()?;

        // Get remaining migrations
        let all_migrations = get_migrations_for_range(state.from_version, state.target_version)?;
        let remaining_migrations: Vec<_> =
            all_migrations.iter().filter(|m| !state.completed_migrations.contains(&m.to_version)).collect();

        if remaining_migrations.is_empty() {
            // All migrations were completed, just need to update version file
            self.write_version_file(state.target_version)?;
            self.cleanup_migration_state()?;
            tracing::info!("✅ Interrupted migration was actually complete, cleaned up state");
            return Ok(());
        }

        tracing::info!("🔄 Resuming migration: {} migration(s) remaining", remaining_migrations.len());

        let mut current_state = state;
        for migration in remaining_migrations {
            if self.abort_flag.load(Ordering::Relaxed) {
                return Err(MigrationError::Aborted);
            }

            tracing::info!(
                "📦 Running migration '{}' (v{} -> v{})",
                migration.name,
                migration.from_version,
                migration.to_version
            );

            let context =
                MigrationContext::new(db, migration.from_version, migration.to_version, self.abort_flag.clone());

            (migration.migrate)(&context).map_err(|e| MigrationError::MigrationStepFailed {
                name: migration.name.to_string(),
                from_version: migration.from_version,
                to_version: migration.to_version,
                message: e.to_string(),
            })?;

            current_state.current_version = migration.to_version;
            current_state.completed_migrations.push(migration.to_version);
            self.save_migration_state(&current_state)?;
            self.write_version_file(migration.to_version)?;

            tracing::info!("✅ Migration '{}' completed", migration.name);
        }

        self.cleanup_migration_state()?;
        tracing::info!("🎉 Resumed migration completed successfully!");
        Ok(())
    }

    /// Acquire the migration lock.
    fn acquire_lock(&self) -> Result<MigrationLock, MigrationError> {
        let lock_path = self.base_path.join(DB_MIGRATION_LOCK);

        if lock_path.exists() {
            // Check if lock is stale (older than 24 hours)
            if let Ok(metadata) = fs::metadata(&lock_path) {
                if let Ok(modified) = metadata.modified() {
                    let age = modified.elapsed().unwrap_or_default();
                    if age > std::time::Duration::from_secs(24 * 60 * 60) {
                        tracing::warn!("Found stale migration lock ({}h old), removing...", age.as_secs() / 3600);
                        fs::remove_file(&lock_path)?;
                    } else {
                        return Err(MigrationError::MigrationInProgress);
                    }
                }
            }
        }

        fs::write(&lock_path, format!("pid:{}\ntime:{}", std::process::id(), chrono::Utc::now().to_rfc3339()))?;
        Ok(MigrationLock { path: lock_path })
    }

    /// Create a backup before migration.
    fn create_backup(&self, db: &DB) -> Result<(), MigrationError> {
        let backup_path = self.base_path.join(BACKUP_DIR_NAME);

        // Remove old backup if exists
        if backup_path.exists() {
            tracing::debug!("Removing old backup at {:?}", backup_path);
            fs::remove_dir_all(&backup_path)?;
        }

        // Use RocksDB's checkpoint feature for consistent backup
        let checkpoint =
            rocksdb::checkpoint::Checkpoint::new(db).map_err(|e| MigrationError::BackupFailed(e.to_string()))?;

        checkpoint.create_checkpoint(&backup_path).map_err(|e| MigrationError::BackupFailed(e.to_string()))?;

        tracing::info!("✅ Backup created at {:?}", backup_path);
        Ok(())
    }

    /// Read the version from the version file.
    fn read_version_file(&self) -> Result<u32, MigrationError> {
        let path = self.base_path.join(DB_VERSION_FILE);
        let content = fs::read_to_string(&path)?;
        content.trim().parse().map_err(|_| MigrationError::InvalidVersionFile)
    }

    /// Write the version to the version file.
    fn write_version_file(&self, version: u32) -> Result<(), MigrationError> {
        let path = self.base_path.join(DB_VERSION_FILE);
        fs::write(&path, version.to_string())?;
        Ok(())
    }

    /// Save migration state for crash recovery.
    fn save_migration_state(&self, state: &MigrationState) -> Result<(), MigrationError> {
        let path = self.base_path.join(DB_MIGRATION_STATE);
        let json = serde_json::to_string_pretty(state)?;
        fs::write(&path, json)?;
        Ok(())
    }

    /// Load migration state if it exists.
    fn load_migration_state(&self) -> Result<Option<MigrationState>, MigrationError> {
        let path = self.base_path.join(DB_MIGRATION_STATE);
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&path)?;
        let state = serde_json::from_str(&content)?;
        Ok(Some(state))
    }

    /// Clean up migration state after successful completion.
    fn cleanup_migration_state(&self) -> Result<(), MigrationError> {
        let state_path = self.base_path.join(DB_MIGRATION_STATE);
        if state_path.exists() {
            fs::remove_file(&state_path)?;
        }
        Ok(())
    }

    /// Get the path to the backup directory.
    pub fn backup_path(&self) -> PathBuf {
        self.base_path.join(BACKUP_DIR_NAME)
    }
}

/// RAII lock guard for migration.
struct MigrationLock {
    path: PathBuf,
}

impl Drop for MigrationLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_test_db(version: Option<u32>) -> (TempDir, PathBuf) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().to_path_buf();

        if let Some(v) = version {
            fs::write(db_path.join(DB_VERSION_FILE), v.to_string()).unwrap();
        }

        (temp_dir, db_path)
    }

    #[test]
    fn test_fresh_database_status() {
        let (temp_dir, db_path) = setup_test_db(None);
        let runner = MigrationRunner::new(&db_path, 9, 8);

        let status = runner.check_status().unwrap();
        assert!(matches!(status, MigrationStatus::FreshDatabase));
        drop(temp_dir);
    }

    #[test]
    fn test_no_migration_needed() {
        let (temp_dir, db_path) = setup_test_db(Some(9));
        let runner = MigrationRunner::new(&db_path, 9, 8);

        let status = runner.check_status().unwrap();
        assert!(matches!(status, MigrationStatus::NoMigrationNeeded));
        drop(temp_dir);
    }

    #[test]
    fn test_database_too_old() {
        let (temp_dir, db_path) = setup_test_db(Some(5));
        let runner = MigrationRunner::new(&db_path, 9, 8);

        let status = runner.check_status().unwrap();
        assert!(matches!(status, MigrationStatus::DatabaseTooOld { current_version: 5, base_version: 8 }));
        drop(temp_dir);
    }

    #[test]
    fn test_database_newer_than_binary() {
        let (temp_dir, db_path) = setup_test_db(Some(10));
        let runner = MigrationRunner::new(&db_path, 9, 8);

        let status = runner.check_status().unwrap();
        assert!(matches!(status, MigrationStatus::DatabaseNewer { db_version: 10, binary_version: 9 }));
        drop(temp_dir);
    }

    #[test]
    fn test_version_file_operations() {
        let (temp_dir, db_path) = setup_test_db(None);
        let runner = MigrationRunner::new(&db_path, 9, 8);

        // Write and read back
        runner.write_version_file(42).unwrap();
        assert_eq!(runner.read_version_file().unwrap(), 42);

        runner.write_version_file(100).unwrap();
        assert_eq!(runner.read_version_file().unwrap(), 100);
        drop(temp_dir);
    }

    #[test]
    fn test_migration_state_operations() {
        let (temp_dir, db_path) = setup_test_db(None);
        let runner = MigrationRunner::new(&db_path, 9, 8);

        // Initially no state
        assert!(runner.load_migration_state().unwrap().is_none());

        // Save and load state
        let state = MigrationState {
            started_at: "2024-01-01T00:00:00Z".to_string(),
            from_version: 8,
            target_version: 9,
            current_version: 8,
            completed_migrations: vec![],
        };
        runner.save_migration_state(&state).unwrap();

        let loaded = runner.load_migration_state().unwrap().unwrap();
        assert_eq!(loaded.from_version, 8);
        assert_eq!(loaded.target_version, 9);

        // Cleanup
        runner.cleanup_migration_state().unwrap();
        assert!(runner.load_migration_state().unwrap().is_none());
        drop(temp_dir);
    }

    #[test]
    fn test_lock_acquisition() {
        let (temp_dir, db_path) = setup_test_db(None);
        let runner = MigrationRunner::new(&db_path, 9, 8);

        // First lock should succeed
        let lock1 = runner.acquire_lock().unwrap();

        // Second lock should fail
        let runner2 = MigrationRunner::new(&db_path, 9, 8);
        assert!(matches!(runner2.acquire_lock(), Err(MigrationError::MigrationInProgress)));

        // After dropping first lock, second should succeed
        drop(lock1);
        let _lock2 = runner2.acquire_lock().unwrap();
        drop(temp_dir);
    }
}

