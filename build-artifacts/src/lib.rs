#![allow(clippy::print_stdout)]

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

    println!("cargo::rerun-if-changed={}", path.display());
    println!("cargo::rustc-env={env}={current_version}");

    Ok(current_version)
}

/// Parent levels just refer to the number of directory levels above the current CARGO_DIR which are
/// needed to reach the directory root. So for example:
///
/// ```text
/// .
/// |
/// |-bootsrapper/            # 1
/// |  |-build.rs             # parent-level = 1
/// |  |-Cargo.toml
/// |
/// |-madara/                 # 4
///    |-crates/              # 3
///        |-client/          # 2
///           |-db/           # 1
///              |-build.rs
///              |-Cargo.toml # parent-level = 4
///
/// ```
///
/// Note that if the env variable `RUST_BUILD_DOCKER` is present at compile time this will disable
/// compiling the artifacts with this build script. This is used to avoid re-compiling artifacts
/// after they have been linked as a docker volume and causing docker-in-docker errors.
pub fn get_or_compile_artifacts(parent_levels: usize) -> Result<(), BuildError> {
    get_or_compile_artifacts_for(parent_levels, &[])
}

/// Same as [`get_or_compile_artifacts`], but takes the list of artifact paths (relative to the
/// repository root) that the calling crate actually needs.
///
/// If every required path already exists on disk (e.g. because it is tracked in git, was
/// extracted by a previous build, or was fetched with `scripts/fetch-artifacts.sh`), Docker is
/// not invoked at all. This keeps fresh source builds working on hosts without a Docker daemon.
///
/// If artifacts are missing and cannot be fetched, this fails fast with an actionable error
/// instead of letting compilation die later on a cryptic `include_bytes!` failure.
pub fn get_or_compile_artifacts_for(parent_levels: usize, required: &[&str]) -> Result<(), BuildError> {
    let (root, version_file_artifacts) = get_paths_artifact(parent_levels)?;

    // Re-run this build script if any required artifact changes or goes missing.
    for path in required {
        println!("cargo::rerun-if-changed={}", root.0.join(path).display());
    }

    // Preflight: if everything this crate needs is already on disk, skip Docker entirely.
    let missing = missing_paths(&root, required);
    if !required.is_empty() && missing.is_empty() {
        return Ok(());
    }

    // if the env variable is present, we return early instead of fetching artifacts
    if std::env::var("RUST_BUILD_DOCKER").is_ok() {
        if missing.is_empty() {
            return Ok(());
        }
        return Err(artifact_help_error(
            &root,
            &version_file_artifacts,
            &missing,
            "RUST_BUILD_DOCKER is set so artifacts are not fetched, but required artifact files are missing",
        ));
    }

    // Preflight: fail fast with instructions if no Docker daemon is reachable, instead of
    // panicking on a missing `docker` binary or running a long `make artifacts` fallback.
    if !docker_available() {
        // Callers that pass an empty `required` slice always reach here with no missing paths
        // to report, so avoid claiming files are missing when none are listed.
        let reason = if required.is_empty() {
            "Docker is not available and artifact files may be missing"
        } else {
            "artifact files are missing and Docker is not available to fetch them"
        };
        return Err(artifact_help_error(&root, &version_file_artifacts, &missing, reason));
    }

    get_artifacts(&root, &version_file_artifacts).or_else(|err| build_artifacts(&root).map_err(|_| err)).map_err(
        |err| {
            let msg = format!("failed to fetch or build artifacts with Docker: {err}");
            artifact_help_error(&root, &version_file_artifacts, &missing, &msg)
        },
    )
}

fn missing_paths(root: &RootDir, required: &[&str]) -> Vec<String> {
    required.iter().filter(|path| !root.0.join(path).exists()).map(|path| path.to_string()).collect()
}

fn docker_available() -> bool {
    std::process::Command::new("docker")
        .args(["version", "--format", "{{.Server.Version}}"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Builds an error that tells the operator exactly how to get the artifacts, with and without
/// Docker. Build script output is shown by cargo when the script fails.
fn artifact_help_error(
    root: &RootDir,
    version_file: &VersionFileArtifacts,
    missing: &[String],
    reason: &str,
) -> BuildError {
    let version = std::fs::read_to_string(version_file.path()).ok().and_then(|content| parse_version(&content).ok());
    let image = match version {
        Some(version) => format!("ghcr.io/madara-alliance/artifacts:{version}"),
        None => "ghcr.io/madara-alliance/artifacts:<version from .artifact-versions.yml>".to_string(),
    };

    let mut msg = format!("{reason}.\n\nRepository root: {}\n", root.0.display());
    if !missing.is_empty() {
        msg.push_str("\nMissing artifact files:\n");
        for path in missing {
            msg.push_str(&format!("  - {path}\n"));
        }
    }
    msg.push_str(&format!(
        "\nTo fix this, from the repository root, do one of:\n\
         \x20 1. Without Docker: run `make fetch-artifacts-no-docker` (or `./scripts/fetch-artifacts.sh`).\n\
         \x20    This downloads {image} over plain HTTPS and extracts it.\n\
         \x20 2. With Docker running: simply re-run the build; the build script pulls {image} automatically.\n\
         \x20 3. To regenerate artifacts from source (requires Docker): run `make artifacts`.\n\
         \nNote: some artifacts (e.g. build-artifacts/cairo_artifacts and build-artifacts/js_tests) are\n\
         tracked in git; `git checkout -- build-artifacts` restores them if they were deleted locally.\n"
    ));

    BuildError::Cmd(msg)
}

fn get_artifacts(root: &RootDir, artifacts: &VersionFileArtifacts) -> Result<(), BuildError> {
    let err_msg = "Failed to download artifacts, make sure that docker is installed";

    let version = get_version(artifacts)?;
    let image = format!("ghcr.io/madara-alliance/artifacts:{version}");
    println!("cargo::warning=fetching artifacts from image: {}", image);

    // Use a unique container name to avoid conflicts in CI environments
    let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let container_name = format!("madara-artifacts-extractor-v{}-{}", version, timestamp);

    let root = &root.0;

    // Download image
    let mut docker = std::process::Command::new("docker");
    let cmd = docker.args(["pull", &image]);
    cmd.status()
        .expect(err_msg)
        .success()
        .then_some(())
        .ok_or_else(|| err_handl(cmd, "Failed to download artifacts"))?;

    // Clean up old artifact extractor containers to prevent accumulation
    // Match containers with pattern: madara-artifacts-extractor-v{version} or madara-artifacts-extractor-v{version}-{timestamp}
    let mut docker = std::process::Command::new("docker");
    docker.args(["ps", "-a", "--format", "{{.Names}}"]);
    if let Ok(output) = docker.output() {
        if output.status.success() {
            let containers = String::from_utf8_lossy(&output.stdout);
            let prefix = format!("madara-artifacts-extractor-v{}", version);
            for container in containers.lines() {
                let container = container.trim();
                // Match containers that start with the prefix (handles both with and without timestamp)
                if !container.is_empty() && container.starts_with(&prefix) {
                    let mut rm_docker = std::process::Command::new("docker");
                    rm_docker.args(["rm", "-f", container]).status().ok();
                }
            }
        }
    }

    // Create extraction container with consistent name
    let mut docker = std::process::Command::new("docker");
    let cmd = docker.args(["create", "--name", &container_name, &image, "do-nothing"]);
    let output = cmd.output().expect(err_msg);

    if !output.status.success() {
        return Err(err_handl(cmd, "Failed to create extraction container"));
    }

    // Copy artifacts from container
    let mut docker = std::process::Command::new("docker");
    let cmd = docker.args(["cp", &format!("{}:/artifacts.tar.gz", container_name), &root.to_string_lossy()]);
    let copy_result = cmd
        .status()
        .expect(err_msg)
        .success()
        .then_some(())
        .ok_or_else(|| err_handl(cmd, "Failed to copy artifacts from extraction container"));

    // Always attempt to remove container, even if copy failed
    let mut docker = std::process::Command::new("docker");
    let cleanup_cmd = docker.args(["rm", "-f", &container_name]);
    let cleanup_result = cleanup_cmd.status();

    // Check if copy failed
    copy_result?;

    // Check if cleanup failed
    if let Ok(status) = cleanup_result {
        if !status.success() {
            println!("cargo::warning=Failed to remove container {}", container_name);
        }
    }

    // Extract artifacts
    let artifacts = std::fs::File::open(root.join("artifacts.tar.gz")).map_err(BuildError::Io)?;
    let decoder = flate2::read::GzDecoder::new(artifacts);
    let mut archive = tar::Archive::new(decoder);
    archive.unpack(root).map_err(BuildError::Io)?;

    Ok(())
}

fn build_artifacts(root: &RootDir) -> Result<(), BuildError> {
    let err_msg = "Failed to build artifacts, make sure that docker and GNU make are installed";
    let root = root.0.to_string_lossy();
    let mut make = std::process::Command::new("sh");
    let cmd = make.arg("-c").arg(format!("yes N | make -C {root} artifacts"));
    cmd.status().expect(err_msg).success().then_some(()).ok_or_else(|| err_handl(cmd, err_msg))
}

fn err_handl(cmd: &mut std::process::Command, msg: &str) -> BuildError {
    println!("carg::warning={msg}: {cmd:?}");
    match cmd.output() {
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr).to_string();
            BuildError::Cmd(stderr)
        }
        Err(_) => BuildError::Cmd(msg.to_string()),
    }
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
