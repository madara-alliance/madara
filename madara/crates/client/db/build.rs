//! Database version management for build-time validation
//!
//! This build script:
//! 1. Reads the current database version from `.db-versions.yml` in project root
//! 2. Injects it as `DB_VERSION` environment variable for runtime checks
//! 3. Ensures version file is well-formatted
//!
//! # File format
//! The version file must be a YAML file with the following structure:
//! ```yaml
//! current_version: 42
//! versions:
//!   - version: 42
//!     pr: 123
//!   - version: 41
//!     pr: 120
//! ```
//!
//! # Environment variables
//! - `CARGO_MANIFEST_DIR`: Set by cargo, path to the current crate
//!
//! # Outputs
//! - `cargo:rustc-env=DB_VERSION=X`: Current database version
//! - `cargo:rerun-if-changed=.db-versions.yml`: Rebuild if version changes
//!
//! # Errors
//! Fails the build if:
//! - Version file is missing or malformed
//! - Version number cannot be parsed
//! - Cannot find project root directory

use std::borrow::Cow;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const DB_VERSION_FILE: &str = ".db-versions.yml";
const PARENT_LEVELS: usize = 4;

#[allow(clippy::print_stderr)]
fn main() {
    if let Err(e) = get_db_version() {
        eprintln!("Failed to get DB version: {}", e);
        std::process::exit(1);
    }
}

#[derive(Debug)]
enum BuildError {
    EnvVar(env::VarError),
    Io(std::io::Error),
    Parse(Cow<'static, str>),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::EnvVar(e) => write!(f, "Environment variable error: {}", e),
            BuildError::Io(e) => write!(f, "IO error: {}", e),
            BuildError::Parse(msg) => write!(f, "Parse error: {}", msg),
        }
    }
}

impl From<env::VarError> for BuildError {
    fn from(e: env::VarError) -> Self {
        BuildError::EnvVar(e)
    }
}

impl From<std::io::Error> for BuildError {
    fn from(e: std::io::Error) -> Self {
        BuildError::Io(e)
    }
}

fn get_db_version() -> Result<(), BuildError> {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR")?;
    let root_dir = get_parents(&PathBuf::from(manifest_dir), PARENT_LEVELS)?;
    let file_path = root_dir.join(DB_VERSION_FILE);

    let content = fs::read_to_string(&file_path).map_err(|e| {
        BuildError::Io(std::io::Error::new(e.kind(), format!("Failed to read {}: {}", file_path.display(), e)))
    })?;

    let current_version = parse_version(&content)?;

    println!("cargo:rerun-if-changed={}", DB_VERSION_FILE);
    println!("cargo:rustc-env=DB_VERSION={}", current_version);

    Ok(())
}

fn parse_version(content: &str) -> Result<u32, BuildError> {
    content
        .lines()
        .find(|line| line.starts_with("current_version:"))
        .ok_or(BuildError::Parse(Cow::Borrowed("Could not find current_version")))?
        .split(':')
        .nth(1)
        .ok_or(BuildError::Parse(Cow::Borrowed("Invalid current_version format")))?
        .trim()
        .parse()
        .map_err(|_| BuildError::Parse(Cow::Borrowed("Could not parse current_version as u32")))
}

fn get_parents(path: &Path, n: usize) -> Result<PathBuf, BuildError> {
    let mut path = path.to_path_buf();
    for _ in 0..n {
        path = path
            .parent()
            .ok_or(BuildError::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "Parent not found")))?
            .to_path_buf();
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_parse_version_valid() {
        let content = "current_version: 42\nother: stuff";
        assert_eq!(parse_version(content).unwrap(), 42);
    }

    #[test]
    fn test_parse_version_invalid_format() {
        let content = "wrong_format";
        assert!(matches!(parse_version(content), Err(BuildError::Parse(_))));
    }

    #[test]
    fn test_get_parents() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("a").join("b").join("c");
        fs::create_dir_all(&path).unwrap();

        let result = get_parents(&path, 2).unwrap();
        assert_eq!(result, temp.path().join("a"));
    }
}
