//! Migration context provided to each migration function.

use rocksdb::{DBWithThreadMode, MultiThreaded};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Type alias for the RocksDB instance (multi-threaded).
type DB = DBWithThreadMode<MultiThreaded>;

/// Progress information for long-running migrations.
#[derive(Debug, Clone)]
pub struct MigrationProgress {
    pub current_step: usize,
    pub total_steps: usize,
    pub message: String,
}

impl MigrationProgress {
    /// Create a new progress report.
    ///
    /// # Parameters
    /// - `current_step`: Zero-based step index. Range: `0..=total_steps` where
    ///   `0` means "starting" and `total_steps` means "completed".
    /// - `total_steps`: Total number of steps in the migration.
    /// - `message`: Human-readable status message.
    ///
    /// # Panics
    /// Panics in debug builds if `current_step > total_steps`.
    pub fn new(current_step: usize, total_steps: usize, message: impl Into<String>) -> Self {
        debug_assert!(
            current_step <= total_steps,
            "current_step ({}) cannot exceed total_steps ({})",
            current_step,
            total_steps
        );
        Self { current_step, total_steps, message: message.into() }
    }
}

/// Type alias for progress callback function.
pub type ProgressCallback = Box<dyn Fn(MigrationProgress) + Send + Sync>;

/// Context provided to each migration function.
pub struct MigrationContext<'a> {
    db: &'a DB,
    from_version: u32,
    to_version: u32,
    progress_callback: Option<ProgressCallback>,
    abort_flag: Arc<AtomicBool>,
}

impl<'a> MigrationContext<'a> {
    pub fn new(db: &'a DB, from_version: u32, to_version: u32, abort_flag: Arc<AtomicBool>) -> Self {
        Self { db, from_version, to_version, progress_callback: None, abort_flag }
    }

    pub fn with_progress_callback(mut self, callback: ProgressCallback) -> Self {
        self.progress_callback = Some(callback);
        self
    }

    pub fn db(&self) -> &DB {
        self.db
    }

    pub fn from_version(&self) -> u32 {
        self.from_version
    }

    pub fn to_version(&self) -> u32 {
        self.to_version
    }

    /// Report progress. Call periodically during long-running migrations.
    pub fn report_progress(&self, progress: MigrationProgress) {
        if let Some(ref callback) = self.progress_callback {
            callback(progress);
        }
    }

    /// Check if migration should abort (graceful shutdown via signal).
    /// Note: Crashes are handled separately via state checkpointing.
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
}
