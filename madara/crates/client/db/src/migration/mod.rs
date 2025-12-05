//! Database migration system for Madara.
//!
//! Automatically runs when opening the database via `MadaraBackend::open_rocksdb`.
//!
//! ## Files Used
//! - `.db-version` - Current database version (single integer)
//! - `.db-migration.lock` - Prevents concurrent migrations
//! - `.db-migration-state` - Checkpoint for crash recovery
//!
//! ## Adding Migrations
//! See [`revisions`] module for instructions.

mod context;
mod error;
mod registry;
pub mod revisions;

pub use context::{MigrationContext, MigrationProgress, ProgressCallback};
pub use error::MigrationError;
pub use registry::{get_migrations, get_migrations_for_range, validate_registry, Migration, MigrationFn};

use rocksdb::{DBWithThreadMode, MultiThreaded};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

type DB = DBWithThreadMode<MultiThreaded>;

// File names - exported for tests
pub const DB_VERSION_FILE: &str = ".db-version";
pub const DB_MIGRATION_LOCK: &str = ".db-migration.lock";
pub const DB_MIGRATION_STATE: &str = ".db-migration-state";
const BACKUP_DIR_NAME: &str = "backup_pre_migration";

#[derive(Debug)]
pub enum MigrationStatus {
    FreshDatabase,
    NoMigrationNeeded,
    MigrationRequired { current_version: u32, target_version: u32, migration_count: usize },
    DatabaseTooOld { current_version: u32, base_version: u32 },
    DatabaseNewer { db_version: u32, binary_version: u32 },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct MigrationState {
    started_at: String,
    from_version: u32,
    target_version: u32,
    current_version: u32,
    completed_migrations: Vec<u32>,
}

/// Main migration orchestrator.
pub struct MigrationRunner {
    base_path: PathBuf,
    required_version: u32,
    base_version: u32,
    abort_flag: Arc<AtomicBool>,
    skip_backup: bool,
}

impl MigrationRunner {
    pub fn new(base_path: &Path, required_version: u32, base_version: u32) -> Self {
        Self {
            base_path: base_path.to_path_buf(),
            required_version,
            base_version,
            abort_flag: Arc::new(AtomicBool::new(false)),
            skip_backup: false, // Default: backup enabled
        }
    }

    /// Skip creating backup before migration.
    /// WARNING: Without backup, there's no recovery if migration fails.
    /// Only use if you have external snapshots/backups.
    pub fn with_skip_backup(mut self, skip: bool) -> Self {
        self.skip_backup = skip;
        self
    }

    /// Signal migration to abort gracefully.
    pub fn abort(&self) {
        self.abort_flag.store(true, Ordering::SeqCst);
    }

    /// Initialize a fresh database by writing the version file.
    pub fn initialize_fresh_database(&self) -> Result<(), MigrationError> {
        self.write_version_file(self.required_version)
    }

    /// Entry point using wrapped storage type.
    pub fn run_migrations_with_storage(&self, storage: &crate::rocksdb::RocksDBStorage) -> Result<(), MigrationError> {
        self.run_migrations(storage.inner_db())
    }

    pub fn check_status(&self) -> Result<MigrationStatus, MigrationError> {
        let version_file = self.base_path.join(DB_VERSION_FILE);

        if !version_file.exists() {
            return Ok(MigrationStatus::FreshDatabase);
        }

        let current_version = self.read_version_file()?;

        if current_version == self.required_version {
            return Ok(MigrationStatus::NoMigrationNeeded);
        }

        if current_version > self.required_version {
            return Ok(MigrationStatus::DatabaseNewer {
                db_version: current_version,
                binary_version: self.required_version,
            });
        }

        if current_version < self.base_version {
            return Ok(MigrationStatus::DatabaseTooOld { current_version, base_version: self.base_version });
        }

        let migrations = get_migrations_for_range(current_version, self.required_version)?;
        Ok(MigrationStatus::MigrationRequired {
            current_version,
            target_version: self.required_version,
            migration_count: migrations.len(),
        })
    }

    /// Main entry point - checks status and runs migrations if needed.
    pub fn run_migrations(&self, db: &DB) -> Result<(), MigrationError> {
        match self.check_status()? {
            MigrationStatus::FreshDatabase => {
                tracing::info!("📦 Fresh database, creating at version {}", self.required_version);
                self.write_version_file(self.required_version)
            }
            MigrationStatus::NoMigrationNeeded => {
                tracing::debug!("✅ Database version {} matches binary", self.required_version);
                Ok(())
            }
            MigrationStatus::DatabaseNewer { db_version, binary_version } => {
                Err(MigrationError::DatabaseNewerThanBinary { db_version, binary_version })
            }
            MigrationStatus::DatabaseTooOld { current_version, base_version } => {
                Err(MigrationError::DatabaseTooOld { current_version, base_version })
            }
            MigrationStatus::MigrationRequired { current_version, target_version, .. } => {
                self.execute_migrations(db, current_version, target_version)
            }
        }
    }

    fn execute_migrations(&self, db: &DB, from_version: u32, to_version: u32) -> Result<(), MigrationError> {
        // Check for interrupted migration and resume if found
        let mut state = if let Some(saved_state) = self.load_migration_state()? {
            // Verify target version matches - if binary was updated, adjust target
            if saved_state.target_version != to_version {
                tracing::warn!(
                    "⚠️  Saved migration target (v{}) differs from binary target (v{}), updating...",
                    saved_state.target_version,
                    to_version
                );
                MigrationState {
                    started_at: saved_state.started_at,
                    from_version: saved_state.from_version,
                    target_version: to_version, // Use current binary's target
                    current_version: saved_state.current_version,
                    completed_migrations: saved_state.completed_migrations,
                }
            } else {
                tracing::warn!(
                    "⚠️  Found interrupted migration from v{} to v{}, resuming from v{}...",
                    saved_state.from_version,
                    saved_state.target_version,
                    saved_state.current_version
                );
                saved_state
            }
        } else {
            MigrationState {
                started_at: chrono::Utc::now().to_rfc3339(),
                from_version,
                target_version: to_version,
                current_version: from_version,
                completed_migrations: vec![],
            }
        };

        let _lock = self.acquire_lock()?;

        let all_migrations = get_migrations_for_range(state.from_version, state.target_version)?;
        let pending: Vec<_> =
            all_migrations.iter().filter(|m| !state.completed_migrations.contains(&m.to_version)).collect();

        if pending.is_empty() {
            // All done, just cleanup
            self.write_version_file(state.target_version)?;
            self.cleanup_migration_state()?;
            tracing::info!("✅ Migration already complete, cleaned up state");
            return Ok(());
        }

        // Only create backup for fresh migrations (not resumes)
        if state.completed_migrations.is_empty() {
            tracing::info!(
                "🔄 Starting migration from v{} to v{} ({} migration(s))",
                from_version,
                to_version,
                pending.len()
            );
            self.save_migration_state(&state)?;

            if self.skip_backup {
                tracing::warn!("⚠️  Skipping backup (--skip-migration-backup). No recovery if migration fails!");
            } else {
                tracing::info!("📸 Creating pre-migration backup...");
                self.create_backup(db)?;
            }
        } else {
            tracing::info!("🔄 Resuming migration: {} migration(s) remaining", pending.len());
        }

        for migration in pending {
            if self.abort_flag.load(Ordering::SeqCst) {
                tracing::warn!("⚠️  Migration aborted by user");
                return Err(MigrationError::Aborted);
            }

            tracing::info!(
                "📦 Running '{}' (v{} -> v{})",
                migration.name,
                migration.from_version,
                migration.to_version
            );

            let context =
                MigrationContext::new(db, migration.from_version, migration.to_version, self.abort_flag.clone())
                    .with_progress_callback(Box::new(|p| {
                        tracing::info!("   [{}/{}] {}", p.current_step, p.total_steps, p.message);
                    }));

            let start = std::time::Instant::now();
            (migration.migrate)(&context).map_err(|e| {
                tracing::error!("❌ Migration '{}' failed: {}", migration.name, e);
                tracing::error!("Restore from backup at: {:?}", self.base_path.join(BACKUP_DIR_NAME));
                MigrationError::MigrationStepFailed {
                    name: migration.name.to_string(),
                    from_version: migration.from_version,
                    to_version: migration.to_version,
                    message: e.to_string(),
                }
            })?;

            tracing::info!("✅ '{}' completed in {:.2}s", migration.name, start.elapsed().as_secs_f64());

            state.current_version = migration.to_version;
            state.completed_migrations.push(migration.to_version);
            self.save_migration_state(&state)?;
            self.write_version_file(migration.to_version)?;
        }

        self.cleanup_migration_state()?;
        self.cleanup_backup()?;
        tracing::info!("🎉 Migration complete! Database now at version {}", to_version);
        Ok(())
    }

    /// Clean up backup after successful migration.
    fn cleanup_backup(&self) -> Result<(), MigrationError> {
        let backup_path = self.base_path.join(BACKUP_DIR_NAME);
        if backup_path.exists() {
            tracing::info!("🗑️  Cleaning up pre-migration backup...");
            fs::remove_dir_all(&backup_path)?;
        }
        Ok(())
    }

    fn acquire_lock(&self) -> Result<MigrationLock, MigrationError> {
        let lock_path = self.base_path.join(DB_MIGRATION_LOCK);

        if lock_path.exists() {
            let is_stale = match fs::metadata(&lock_path).and_then(|m| m.modified()) {
                Ok(modified) => match modified.elapsed() {
                    Ok(age) => age > std::time::Duration::from_secs(24 * 60 * 60),
                    Err(e) => {
                        // System time went backwards, treat as stale
                        tracing::warn!("System time error checking lock age: {}, treating as stale", e);
                        true
                    }
                },
                Err(e) => {
                    // Can't determine age, treat as stale
                    tracing::warn!("Cannot read lock metadata: {}, treating as stale", e);
                    true
                }
            };

            if is_stale {
                tracing::warn!("Removing stale migration lock");
                fs::remove_file(&lock_path)?;
            } else {
                return Err(MigrationError::MigrationInProgress);
            }
        }

        fs::write(&lock_path, format!("pid:{}\ntime:{}", std::process::id(), chrono::Utc::now().to_rfc3339()))?;
        Ok(MigrationLock { path: lock_path })
    }

    /// Creates a backup using RocksDB checkpoint (uses hard links, very fast).
    fn create_backup(&self, db: &DB) -> Result<(), MigrationError> {
        let backup_path = self.base_path.join(BACKUP_DIR_NAME);

        if backup_path.exists() {
            fs::remove_dir_all(&backup_path)?;
        }

        // Checkpoint creates hard links to SST files - fast and space-efficient
        let checkpoint =
            rocksdb::checkpoint::Checkpoint::new(db).map_err(|e| MigrationError::BackupFailed(e.to_string()))?;
        checkpoint.create_checkpoint(&backup_path).map_err(|e| MigrationError::BackupFailed(e.to_string()))?;

        tracing::info!("✅ Backup created at {:?}", backup_path);
        Ok(())
    }

    // Version file operations - public for testing
    pub fn read_version_file(&self) -> Result<u32, MigrationError> {
        let content = fs::read_to_string(self.base_path.join(DB_VERSION_FILE))?;
        content.trim().parse().map_err(|_| MigrationError::InvalidVersionFile)
    }

    pub fn write_version_file(&self, version: u32) -> Result<(), MigrationError> {
        fs::write(self.base_path.join(DB_VERSION_FILE), version.to_string())?;
        Ok(())
    }

    fn save_migration_state(&self, state: &MigrationState) -> Result<(), MigrationError> {
        fs::write(self.base_path.join(DB_MIGRATION_STATE), serde_json::to_string_pretty(state)?)?;
        Ok(())
    }

    fn load_migration_state(&self) -> Result<Option<MigrationState>, MigrationError> {
        let path = self.base_path.join(DB_MIGRATION_STATE);
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(serde_json::from_str(&fs::read_to_string(&path)?)?))
    }

    fn cleanup_migration_state(&self) -> Result<(), MigrationError> {
        let path = self.base_path.join(DB_MIGRATION_STATE);
        if path.exists() {
            fs::remove_file(&path)?;
        }
        Ok(())
    }

    pub fn backup_path(&self) -> PathBuf {
        self.base_path.join(BACKUP_DIR_NAME)
    }
}

/// RAII lock guard - removes lock file on drop.
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
    fn test_check_status_variants() {
        // Fresh
        let (_t, path) = setup_test_db(None);
        assert!(matches!(MigrationRunner::new(&path, 9, 8).check_status().unwrap(), MigrationStatus::FreshDatabase));

        // Same version
        let (_t, path) = setup_test_db(Some(9));
        assert!(matches!(
            MigrationRunner::new(&path, 9, 8).check_status().unwrap(),
            MigrationStatus::NoMigrationNeeded
        ));

        // Too old
        let (_t, path) = setup_test_db(Some(5));
        assert!(matches!(
            MigrationRunner::new(&path, 9, 8).check_status().unwrap(),
            MigrationStatus::DatabaseTooOld { current_version: 5, base_version: 8 }
        ));

        // Newer than binary
        let (_t, path) = setup_test_db(Some(10));
        assert!(matches!(
            MigrationRunner::new(&path, 9, 8).check_status().unwrap(),
            MigrationStatus::DatabaseNewer { db_version: 10, binary_version: 9 }
        ));
    }

    #[test]
    fn test_version_file_roundtrip() {
        let (_t, path) = setup_test_db(None);
        let runner = MigrationRunner::new(&path, 9, 8);

        runner.write_version_file(42).unwrap();
        assert_eq!(runner.read_version_file().unwrap(), 42);
    }

    #[test]
    fn test_migration_state_roundtrip() {
        let (_t, path) = setup_test_db(None);
        let runner = MigrationRunner::new(&path, 9, 8);

        assert!(runner.load_migration_state().unwrap().is_none());

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

        runner.cleanup_migration_state().unwrap();
        assert!(runner.load_migration_state().unwrap().is_none());
    }

    #[test]
    fn test_lock_prevents_concurrent_access() {
        let (_t, path) = setup_test_db(None);
        let runner1 = MigrationRunner::new(&path, 9, 8);
        let runner2 = MigrationRunner::new(&path, 9, 8);

        let _lock = runner1.acquire_lock().unwrap();
        assert!(matches!(runner2.acquire_lock(), Err(MigrationError::MigrationInProgress)));
    }
}
