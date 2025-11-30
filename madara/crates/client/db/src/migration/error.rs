//! Migration error types.

#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error(
        "Database version {db_version} is newer than binary version {binary_version}. \
        Please upgrade to a newer version of the binary."
    )]
    DatabaseNewerThanBinary { db_version: u32, binary_version: u32 },

    #[error(
        "Database version {current_version} is older than minimum supported version {base_version}. \
        Please delete the database directory and resync from scratch."
    )]
    DatabaseTooOld { current_version: u32, base_version: u32 },

    /// Migration registry is missing a required migration. This is a developer error -
    /// ensure all migrations from base_version to current_version are registered.
    #[error(
        "Migration registry bug: no migration registered for v{from} -> v{to}. \
        Check that all migrations are registered in registry.rs"
    )]
    NoMigrationPath { from: u32, to: u32 },

    #[error(
        "Migration lock file exists - another migration may be in progress. \
        If no other instance is running, delete .db-migration.lock manually."
    )]
    MigrationInProgress,

    #[error("Failed to create pre-migration backup: {0}")]
    BackupFailed(String),

    #[error("Invalid database version file format. Expected a single integer.")]
    InvalidVersionFile,

    #[error("Migration '{name}' (v{from_version} -> v{to_version}) failed: {message}")]
    MigrationStepFailed { name: String, from_version: u32, to_version: u32, message: String },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("RocksDB error: {0}")]
    RocksDb(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Migration aborted by user")]
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
        assert!(err.to_string().contains("10") && err.to_string().contains("9"));

        let err = MigrationError::NoMigrationPath { from: 8, to: 9 };
        assert!(err.to_string().contains("registry bug"));
    }
}
