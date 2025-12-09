#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error("Database version {db_version} is newer than binary supports ({binary_version}). Upgrade the binary.")]
    DatabaseNewerThanBinary { db_version: u32, binary_version: u32 },

    #[error("Database version {current_version} too old (minimum: {base_version}). Delete and resync.")]
    DatabaseTooOld { current_version: u32, base_version: u32 },

    #[error("No migration registered for v{from} -> v{to}")]
    NoMigrationPath { from: u32, to: u32 },

    #[error("Migration lock exists - another migration may be in progress")]
    MigrationInProgress,

    #[error("Failed to create backup: {message}")]
    BackupFailed {
        message: &'static str,
        #[source]
        source: rocksdb::Error,
    },

    #[error("Insufficient disk space for backup: need {required} bytes, only {available} bytes available")]
    InsufficientDiskSpace { required: u64, available: u64 },

    #[error("Invalid version file format")]
    InvalidVersionFile,

    #[error("Migration '{name}' (v{from_version} -> v{to_version}) failed: {message}")]
    MigrationStepFailed { name: String, from_version: u32, to_version: u32, message: String },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("RocksDB error: {0}")]
    RocksDb(#[from] rocksdb::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Migration aborted")]
    Aborted,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = MigrationError::DatabaseNewerThanBinary { db_version: 10, binary_version: 9 };
        assert!(err.to_string().contains("10") && err.to_string().contains("9"));

        let err = MigrationError::NoMigrationPath { from: 8, to: 9 };
        assert!(err.to_string().contains("v8") && err.to_string().contains("v9"));
    }
}
