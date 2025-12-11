//! Migration context provided to each migration function.

use rocksdb::{DBWithThreadMode, MultiThreaded};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

type DB = DBWithThreadMode<MultiThreaded>;

#[derive(Debug, Clone)]
pub struct MigrationProgress {
    pub current_step: usize,
    pub total_steps: usize,
    pub message: String,
}

impl MigrationProgress {
    pub fn new(current_step: usize, total_steps: usize, message: impl Into<String>) -> Self {
        debug_assert!(current_step <= total_steps);
        Self { current_step, total_steps, message: message.into() }
    }
}

pub type ProgressCallback = Box<dyn Fn(MigrationProgress) + Send + Sync>;

/// Context provided to each migration function.
pub struct MigrationContext<'a> {
    db: &'a DB,
    progress_callback: Option<ProgressCallback>,
    abort_flag: Arc<AtomicBool>,
}

impl<'a> MigrationContext<'a> {
    pub fn new(db: &'a DB, abort_flag: Arc<AtomicBool>) -> Self {
        Self { db, progress_callback: None, abort_flag }
    }

    pub fn with_progress_callback(mut self, callback: ProgressCallback) -> Self {
        self.progress_callback = Some(callback);
        self
    }

    pub fn db(&self) -> &DB {
        self.db
    }

    pub fn report_progress(&self, progress: MigrationProgress) {
        if let Some(ref callback) = self.progress_callback {
            callback(progress);
        }
    }

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
