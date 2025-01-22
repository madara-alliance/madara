//! Database version compatibility checker
//!
//! This module ensures database version compatibility with the current binary.
//! The version check prevents data corruption from version mismatches between
//! database files and binary versions.
//!
//! # Version File
//! The version is stored in a `.db-version` file in the database directory.
//! This file contains a single number representing the database version.
//!

use std::fs;
use std::path::Path;

/// Database version from build-time, injected by build.rs
const REQUIRED_DB_VERSION: &str = env!("DB_VERSION");

/// File name for database version
/// This file contains a single number representing the database version
const DB_VERSION_FILE: &str = ".db-version";

/// Errors that can occur during version checking
#[derive(Debug, thiserror::Error)]
pub enum DbVersionError {
    /// The database version doesn't match the binary version
    #[error(
        "Database version {db_version} is not compatible with current binary. Expected version {required_version}"
    )]
    IncompatibleVersion {
        /// Version found in database
        db_version: u32,
        /// Version required by binary
        required_version: u32,
    },

    /// Error reading or writing the version file
    #[error("Failed to read database version: {0}")]
    VersionReadError(String),
}

/// Checks database version compatibility with current binary.
///
/// # Arguments
/// * `path` - Path to the database directory
///
/// # Returns
/// * `Ok(None)` - New database created with current version
/// * `Ok(Some(version))` - Existing database with compatible version
/// * `Err(DbVersionError)` - Version mismatch or IO error
///
/// # Examples
/// ```ignore
/// use std::path::Path;
/// use crate::db_version::check_db_version;
///
/// let db_path = Path::new("test_db");
/// match check_db_version(db_path) {
///     Ok(None) => println!("Created new database"),
///     Ok(Some(v)) => println!("Database version {} is compatible", v),
///     Err(e) => eprintln!("Error: {}", e),
/// }
/// ```
///
pub fn check_db_version(path: &Path) -> Result<Option<u32>, DbVersionError> {
    let required_db_version =
        REQUIRED_DB_VERSION.parse::<u32>().expect("REQUIRED_DB_VERSION is checked at compile time");

    // Create directory if it doesn't exist
    if !path.exists() {
        fs::create_dir_all(path).map_err(|e| DbVersionError::VersionReadError(e.to_string()))?;
    }

    let file_path = path.join(DB_VERSION_FILE);
    if !file_path.exists() {
        // Initialize new database with current version
        fs::write(&file_path, REQUIRED_DB_VERSION).map_err(|e| DbVersionError::VersionReadError(e.to_string()))?;
        Ok(None)
    } else {
        // Check existing database version
        let version = fs::read_to_string(&file_path).map_err(|e| DbVersionError::VersionReadError(e.to_string()))?;
        let version = version.trim().parse::<u32>().map_err(|_| DbVersionError::VersionReadError(version))?;

        if version != required_db_version {
            return Err(DbVersionError::IncompatibleVersion {
                db_version: version,
                required_version: required_db_version,
            });
        }
        Ok(Some(version))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Helper function to create a test database directory
    fn setup_test_db() -> TempDir {
        TempDir::new().unwrap()
    }

    #[test]
    fn test_new_database() {
        let temp_dir = setup_test_db();
        let result = check_db_version(temp_dir.path()).unwrap();
        assert!(result.is_none());

        // Verify version file was created
        let version_file = temp_dir.path().join(DB_VERSION_FILE);
        assert!(version_file.exists());

        // Verify content
        let content = fs::read_to_string(version_file).unwrap();
        assert_eq!(content, REQUIRED_DB_VERSION);
    }

    #[test]
    fn test_compatible_version() {
        let temp_dir = setup_test_db();
        let version_file = temp_dir.path().join(DB_VERSION_FILE);

        // Create version file with current version
        fs::write(&version_file, REQUIRED_DB_VERSION).unwrap();

        let result = check_db_version(temp_dir.path()).unwrap();
        assert_eq!(result, Some(REQUIRED_DB_VERSION.parse().unwrap()));
    }

    #[test]
    fn test_incompatible_version() {
        let temp_dir = setup_test_db();
        let version_file = temp_dir.path().join(DB_VERSION_FILE);

        // Create version file with different version
        let incompatible_version = REQUIRED_DB_VERSION.parse::<u32>().unwrap().checked_add(1).unwrap().to_string();
        fs::write(version_file, incompatible_version).unwrap();

        let err = check_db_version(temp_dir.path()).unwrap_err();
        assert!(matches!(err, DbVersionError::IncompatibleVersion { .. }));
    }

    #[test]
    fn test_invalid_version_format() {
        let temp_dir = setup_test_db();
        let version_file = temp_dir.path().join(DB_VERSION_FILE);

        // Create version file with invalid content
        fs::write(version_file, "invalid").unwrap();

        let err = check_db_version(temp_dir.path()).unwrap_err();
        assert!(matches!(err, DbVersionError::VersionReadError(..)));
    }

    #[test]
    fn test_creates_missing_directory() {
        let temp_dir = setup_test_db();
        let db_path = temp_dir.path().join(DB_VERSION_FILE);

        let result = check_db_version(&db_path).unwrap();
        assert!(result.is_none());
        assert!(db_path.exists());
        assert!(db_path.join(".db-version").exists());
    }
}
