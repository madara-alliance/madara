//! Migration-specific error types.
//!
//! This module defines all possible errors that can occur during database migrations.

/// Errors that can occur during database migration.
#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    /// Database version is newer than what the binary supports.
    /// User needs to upgrade to a newer binary version.
    #[error(
        "Database version {db_version} is newer than binary version {binary_version}. \
        Please upgrade to a newer version of the binary."
    )]
    DatabaseNewerThanBinary {
        /// Version found in database
        db_version: u32,
        /// Version expected by binary
        binary_version: u32,
    },

    /// Database version is older than the minimum supported version.
    /// User needs to resync the database from scratch.
    #[error(
        "Database version {current_version} is older than minimum supported version {base_version}. \
        Please delete the database directory and resync from scratch."
    )]
    DatabaseTooOld {
        /// Version found in database
        current_version: u32,
        /// Minimum version supported for migration
        base_version: u32,
    },

    /// No migration path exists between the versions.
    /// This indicates a bug in the migration registry.
    #[error("No migration path found from version {from} to version {to}. This is a bug.")]
    NoMigrationPath {
        /// Source version
        from: u32,
        /// Target version
        to: u32,
    },

    /// Another migration is already in progress.
    /// This can happen if a previous migration crashed and left a lock file.
    #[error(
        "Migration is already in progress. If a previous migration crashed, \
        remove the .db-migration.lock file manually after ensuring no other instance is running."
    )]
    MigrationInProgress,

    /// Failed to create a backup before migration.
    #[error("Failed to create pre-migration backup: {0}")]
    BackupFailed(String),

    /// The version file has an invalid format.
    #[error("Invalid database version file format. Expected a single integer.")]
    InvalidVersionFile,

    /// A specific migration step failed.
    #[error("Migration '{name}' (v{from_version} -> v{to_version}) failed: {message}")]
    MigrationStepFailed {
        /// Name of the migration that failed
        name: String,
        /// Source version
        from_version: u32,
        /// Target version
        to_version: u32,
        /// Error message
        message: String,
    },

    /// IO error during migration.
    #[error("IO error during migration: {0}")]
    Io(#[from] std::io::Error),

    /// RocksDB error during migration.
    #[error("RocksDB error during migration: {0}")]
    RocksDb(String),

    /// Serialization/deserialization error.
    #[error("Serialization error during migration: {0}")]
    Serialization(String),

    /// Migration was aborted (e.g., by user signal).
    #[error("Migration was aborted")]
    Aborted,
}

impl From<rocksdb::Error> for MigrationError {
    fn from(e: rocksdb::Error) -> Self {
        MigrationError::RocksDb(e.to_string())
    }
}

impl From<serde_json::Error> for MigrationError {
    fn from(e: serde_json::Error) -> Self {
        MigrationError::Serialization(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = MigrationError::DatabaseNewerThanBinary { db_version: 10, binary_version: 9 };
        assert!(err.to_string().contains("10"));
        assert!(err.to_string().contains("9"));

        let err = MigrationError::DatabaseTooOld { current_version: 5, base_version: 8 };
        assert!(err.to_string().contains("5"));
        assert!(err.to_string().contains("8"));

        let err = MigrationError::MigrationStepFailed {
            name: "test_migration".to_string(),
            from_version: 8,
            to_version: 9,
            message: "something went wrong".to_string(),
        };
        assert!(err.to_string().contains("test_migration"));
        assert!(err.to_string().contains("something went wrong"));
    }
}

