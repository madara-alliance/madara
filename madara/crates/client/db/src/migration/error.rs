//! Migration error types.

#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error("DB v{db_version} > binary v{binary_version}. Upgrade the binary.")]
    DatabaseNewerThanBinary { db_version: u32, binary_version: u32 },

    #[error("DB v{current_version} < min v{base_version}. Delete DB and resync.")]
    DatabaseTooOld { current_version: u32, base_version: u32 },

    #[error("No migration v{from}→v{to}. Check registry.rs.")]
    NoMigrationPath { from: u32, to: u32 },

    #[error("Migration lock exists. Delete .db-migration.lock if no other instance running.")]
    MigrationInProgress,

    #[error("Backup failed: {0}")]
    BackupFailed(String),

    #[error("Invalid .db-version file")]
    InvalidVersionFile,

    #[error("Migration '{name}' (v{from_version}→v{to_version}) failed: {message}")]
    MigrationStepFailed { name: String, from_version: u32, to_version: u32, message: String },

    #[error("IO: {0}")]
    Io(#[from] std::io::Error),

    #[error("RocksDB: {0}")]
    RocksDb(String),

    #[error("Serialization: {0}")]
    Serialization(String),

    #[error("Aborted")]
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
