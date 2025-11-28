//! Migration context provided to each migration function.
//!
//! The context provides access to the database and utilities needed during migration.

use rocksdb::DB;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Progress information for long-running migrations.
#[derive(Debug, Clone)]
pub struct MigrationProgress {
    /// Current step number (1-indexed)
    pub current_step: usize,
    /// Total number of steps (0 if unknown)
    pub total_steps: usize,
    /// Human-readable progress message
    pub message: String,
}

impl MigrationProgress {
    /// Create a new progress update.
    pub fn new(current_step: usize, total_steps: usize, message: impl Into<String>) -> Self {
        Self { current_step, total_steps, message: message.into() }
    }

    /// Create a progress update with unknown total.
    pub fn indeterminate(current_step: usize, message: impl Into<String>) -> Self {
        Self { current_step, total_steps: 0, message: message.into() }
    }
}

/// Type alias for progress callback function.
pub type ProgressCallback = Box<dyn Fn(MigrationProgress) + Send + Sync>;

/// Context provided to each migration function.
///
/// This struct provides everything a migration needs:
/// - Access to the RocksDB instance for reading/writing data
/// - Version information (from/to)
/// - Progress reporting callback
/// - Abort checking for graceful shutdown
pub struct MigrationContext<'a> {
    /// Direct access to RocksDB for low-level operations.
    /// Migrations should use this to read/write data.
    db: &'a DB,

    /// Version being migrated FROM.
    from_version: u32,

    /// Version being migrated TO.
    to_version: u32,

    /// Optional callback for reporting progress.
    /// Useful for long-running migrations to show progress to the user.
    progress_callback: Option<ProgressCallback>,

    /// Flag to check if migration should be aborted.
    /// Set by signal handlers for graceful shutdown.
    abort_flag: Arc<AtomicBool>,
}

impl<'a> MigrationContext<'a> {
    /// Create a new migration context.
    pub fn new(
        db: &'a DB,
        from_version: u32,
        to_version: u32,
        abort_flag: Arc<AtomicBool>,
    ) -> Self {
        Self { db, from_version, to_version, progress_callback: None, abort_flag }
    }

    /// Set the progress callback.
    pub fn with_progress_callback(mut self, callback: ProgressCallback) -> Self {
        self.progress_callback = Some(callback);
        self
    }

    /// Get a reference to the RocksDB instance.
    pub fn db(&self) -> &DB {
        self.db
    }

    /// Get the source version (migrating FROM).
    pub fn from_version(&self) -> u32 {
        self.from_version
    }

    /// Get the target version (migrating TO).
    pub fn to_version(&self) -> u32 {
        self.to_version
    }

    /// Report progress during migration.
    ///
    /// Call this periodically during long-running migrations to keep the user informed.
    pub fn report_progress(&self, progress: MigrationProgress) {
        if let Some(ref callback) = self.progress_callback {
            callback(progress);
        }
    }

    /// Check if the migration should be aborted.
    ///
    /// Long-running migrations should check this periodically and return early
    /// if true to allow graceful shutdown.
    pub fn should_abort(&self) -> bool {
        self.abort_flag.load(Ordering::Relaxed)
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_progress_new() {
        let progress = MigrationProgress::new(5, 10, "Processing items");
        assert_eq!(progress.current_step, 5);
        assert_eq!(progress.total_steps, 10);
        assert_eq!(progress.message, "Processing items");
    }

    #[test]
    fn test_progress_indeterminate() {
        let progress = MigrationProgress::indeterminate(42, "Still working...");
        assert_eq!(progress.current_step, 42);
        assert_eq!(progress.total_steps, 0);
    }
}

