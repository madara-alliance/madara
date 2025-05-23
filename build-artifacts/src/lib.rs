const VERSION_FILE_DB: &str = ".db-versions.yml";
const VERSION_FILE_ARTIFACTS: &str = ".artifact-versions.yml";

#[derive(Debug)]
pub enum BuildError {
    EnvVar(std::env::VarError),
    Io(std::io::Error),
    Parse(std::borrow::Cow<'static, str>),
    Cmd(String),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::EnvVar(e) => write!(f, "Environment variable error: {e}"),
            BuildError::Io(e) => write!(f, "IO error: {e}"),
            BuildError::Parse(msg) => write!(f, "Parse error: {msg}"),
            BuildError::Cmd(msg) => write!(f, "Run error: {msg}"),
        }
    }
}

impl From<std::env::VarError> for BuildError {
    fn from(e: std::env::VarError) -> Self {
        BuildError::EnvVar(e)
    }
}

impl From<std::io::Error> for BuildError {
    fn from(e: std::io::Error) -> Self {
        BuildError::Io(e)
    }
}

pub struct RootDir(std::path::PathBuf);
pub struct VersionFileDb(std::path::PathBuf);
pub struct VersionFileArtifacts(std::path::PathBuf);

pub trait VersionFile {
    fn path(&self) -> &std::path::PathBuf;
    fn env(&self) -> &'static str;
}

impl VersionFile for VersionFileDb {
    fn path(&self) -> &std::path::PathBuf {
        &self.0
    }

    fn env(&self) -> &'static str {
        "VERSION_DB"
    }
}

impl VersionFile for VersionFileArtifacts {
    fn path(&self) -> &std::path::PathBuf {
        &self.0
    }

    fn env(&self) -> &'static str {
        "VERSION_ARTIFACTS"
    }
}

pub fn get_paths_db(parent_levels: usize) -> Result<(RootDir, VersionFileDb), BuildError> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")?;
    let root_dir = get_parents(&std::path::PathBuf::from(manifest_dir), parent_levels)?;
    let version_file_db = root_dir.join(VERSION_FILE_DB);

    Ok((RootDir(root_dir), VersionFileDb(version_file_db)))
}

pub fn get_paths_artifact(parent_levels: usize) -> Result<(RootDir, VersionFileArtifacts), BuildError> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")?;
    let root_dir = get_parents(&std::path::PathBuf::from(manifest_dir), parent_levels)?;
    let version_file_artifacts = root_dir.join(VERSION_FILE_ARTIFACTS);

    Ok((RootDir(root_dir), VersionFileArtifacts(version_file_artifacts)))
}

pub fn get_version(version_file: &impl VersionFile) -> Result<u32, BuildError> {
    let path = version_file.path();
    let env = version_file.env();
    let content = std::fs::read_to_string(path).map_err(|e| {
        BuildError::Io(std::io::Error::new(e.kind(), format!("Failed to read {}: {}", path.display(), e)))
    })?;

    let current_version = parse_version(&content)?;

    println!("cargo:rerun-if-changed={}", path.display());
    println!("cargo:rustc-env={env}={current_version}");

    Ok(current_version)
}

pub fn get_or_compile_artifacts(parent_levels: usize) -> Result<(), BuildError> {
    let (root, version_file_artifacts) = get_paths_artifact(parent_levels)?;

    if let Ok(true) = std::fs::exists(version_file_artifacts.path()) {
        return Ok(());
    }

    get_artifacts(&root, &version_file_artifacts).or_else(|_error| build_artifacts(&root))
}

fn get_artifacts(root: &RootDir, artifacts: &VersionFileArtifacts) -> Result<(), BuildError> {
    let err_msg = "Failed to download artifacts, make sure that docker is installed";
    let err_handl = |cmd: &mut std::process::Command| {
        BuildError::Cmd(
            cmd.output().map(|o| String::from_utf8_lossy(&o.stderr).to_string()).unwrap_or(err_msg.to_string()),
        )
    };

    let version = get_version(artifacts)?;
    let image = format!("ghcr.io/madara-alliance/artifacts:{version}");
    let mut docker = std::process::Command::new("docker");

    let root = &root.0;
    let path_artifacts = artifacts.path();

    // Download image
    let cmd = docker.args(["pull", &image]);
    cmd.status().expect(err_msg).success().then_some(()).ok_or_else(|| err_handl(cmd))?;

    // Create extraction container
    let cmd = docker.args(["create", "--name", "artifacts", &image, "do-nothing"]);
    cmd.status().expect(err_msg).success().then_some(()).ok_or_else(|| err_handl(cmd))?;

    // Copy artifacts from container
    let cmd = docker.args(["cp", "artifacts:/artifacts.tar.gz", &root.to_string_lossy()]);
    cmd.status().expect(err_msg).success().then_some(()).ok_or_else(|| err_handl(cmd))?;

    // Extract artifacts
    let artifacts = std::fs::File::open(root.join("artifacts.tar.gz")).map_err(BuildError::Io)?;
    let decoder = flate2::read::GzDecoder::new(artifacts);
    let mut archive = tar::Archive::new(decoder);
    archive.unpack(path_artifacts).map_err(BuildError::Io)?;

    // Remove container
    let cmd = docker.args(["rm", "artifacts"]);
    cmd.status().expect(err_msg).success().then_some(()).ok_or_else(|| err_handl(cmd))?;

    Ok(())
}

fn build_artifacts(root: &RootDir) -> Result<(), BuildError> {
    let err_msg = "Failed to build artifacts, make sure that docker and GNU make are installed";
    let err_handl = |cmd: &mut std::process::Command| {
        BuildError::Cmd(
            cmd.output().map(|o| String::from_utf8_lossy(&o.stderr).to_string()).unwrap_or(err_msg.to_string()),
        )
    };

    let mut make = std::process::Command::new("make");
    let cmd = make.args(["-C", &root.0.to_string_lossy(), "artifacts"]);
    cmd.status().expect(err_msg).success().then_some(()).ok_or_else(|| err_handl(cmd))
}

fn parse_version(content: &str) -> Result<u32, BuildError> {
    content
        .lines()
        .find(|line| line.starts_with("current_version:"))
        .ok_or(BuildError::Parse(std::borrow::Cow::Borrowed("Could not find current_version")))?
        .split(':')
        .nth(1)
        .ok_or(BuildError::Parse(std::borrow::Cow::Borrowed("Invalid current_version format")))?
        .trim()
        .parse()
        .map_err(|_| BuildError::Parse(std::borrow::Cow::Borrowed("Could not parse current_version as u32")))
}

fn get_parents(path: &std::path::Path, n: usize) -> Result<std::path::PathBuf, BuildError> {
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
