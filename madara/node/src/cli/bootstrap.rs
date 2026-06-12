//! Snapshot bootstrap CLI and importer.

use anyhow::{bail, ensure, Context};
use clap::Args;
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use mp_chain_config::ChainConfig;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use starknet_types_core::felt::Felt;
use std::{
    fs::{self, File},
    io::{BufReader, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
use url::Url;

/// Parameters for importing a pre-synced Madara base-path snapshot before the database opens.
#[derive(Clone, Debug, Args, Default, Deserialize, Serialize)]
pub struct BootstrapParams {
    /// Import a pre-synced Madara base-path snapshot archive before opening the database.
    ///
    /// The archive must be a `.tar.gz` whose root contains the Madara base-path
    /// contents, including `db/` and `.db-version`.
    #[arg(env = "MADARA_BOOTSTRAP_SNAPSHOT", long, value_name = "TAR.GZ", conflicts_with = "bootstrap_snapshot_url")]
    #[serde(default)]
    pub bootstrap_snapshot: Option<PathBuf>,

    /// Download and import a pre-synced Madara base-path snapshot archive before opening the database.
    ///
    /// The URL must point to a `.tar.gz` whose root contains the Madara base-path
    /// contents, including `db/` and `.db-version`.
    #[arg(
        env = "MADARA_BOOTSTRAP_SNAPSHOT_URL",
        long,
        value_name = "URL",
        conflicts_with_all = ["bootstrap_snapshot", "bootstrap_snapshot_manifest"]
    )]
    #[serde(default)]
    pub bootstrap_snapshot_url: Option<Url>,

    /// Manifest JSON for `--bootstrap-snapshot`.
    ///
    /// If omitted, Madara looks for `<TAR.GZ>.manifest.json` next to the archive.
    #[arg(env = "MADARA_BOOTSTRAP_SNAPSHOT_MANIFEST", long, value_name = "JSON")]
    #[serde(default)]
    pub bootstrap_snapshot_manifest: Option<PathBuf>,

    /// Manifest JSON URL for `--bootstrap-snapshot-url`.
    ///
    /// If omitted, Madara appends `.manifest.json` to the snapshot URL path.
    #[arg(
        env = "MADARA_BOOTSTRAP_SNAPSHOT_MANIFEST_URL",
        long,
        value_name = "URL",
        requires = "bootstrap_snapshot_url",
        conflicts_with = "bootstrap_snapshot_manifest"
    )]
    #[serde(default)]
    pub bootstrap_snapshot_manifest_url: Option<Url>,

    /// Create a bootstrap snapshot archive from `--base-path` and exit.
    ///
    /// Madara opens the database, creates a RocksDB checkpoint, archives that
    /// checkpoint with `.db-version`, writes the manifest, then exits.
    #[arg(
        env = "MADARA_CREATE_BOOTSTRAP_SNAPSHOT",
        long,
        value_name = "TAR.GZ",
        conflicts_with_all = ["bootstrap_snapshot", "bootstrap_snapshot_url"]
    )]
    #[serde(default)]
    pub create_bootstrap_snapshot: Option<PathBuf>,

    /// Manifest JSON to write for `--create-bootstrap-snapshot`.
    ///
    /// If omitted, Madara writes `<TAR.GZ>.manifest.json` next to the archive.
    #[arg(env = "MADARA_CREATE_BOOTSTRAP_SNAPSHOT_MANIFEST", long, value_name = "JSON")]
    #[serde(default)]
    pub create_bootstrap_snapshot_manifest: Option<PathBuf>,
}

impl BootstrapParams {
    /// Imports the configured snapshot into `base_path` if snapshot bootstrap is enabled.
    pub fn import_snapshot_if_requested(
        &self,
        base_path: &Path,
        chain_config: &Arc<ChainConfig>,
    ) -> anyhow::Result<Option<BootstrapSnapshotManifest>> {
        match (&self.bootstrap_snapshot, &self.bootstrap_snapshot_url) {
            (Some(snapshot_path), None) => {
                ensure!(
                    self.bootstrap_snapshot_manifest_url.is_none(),
                    "--bootstrap-snapshot-manifest-url requires --bootstrap-snapshot-url"
                );
                import_snapshot(snapshot_path, self.bootstrap_snapshot_manifest.as_deref(), base_path, chain_config)
                    .map(Some)
            }
            (None, Some(snapshot_url)) => {
                ensure!(
                    self.bootstrap_snapshot_manifest.is_none(),
                    "--bootstrap-snapshot-manifest cannot be used with --bootstrap-snapshot-url"
                );
                import_remote_snapshot(
                    snapshot_url,
                    self.bootstrap_snapshot_manifest_url.as_ref(),
                    base_path,
                    chain_config,
                )
                .map(Some)
            }
            (Some(_), Some(_)) => {
                bail!("Only one of --bootstrap-snapshot or --bootstrap-snapshot-url can be provided")
            }
            (None, None) => {
                ensure!(
                    self.bootstrap_snapshot_manifest_url.is_none(),
                    "--bootstrap-snapshot-manifest-url requires --bootstrap-snapshot-url"
                );
                Ok(None)
            }
        }
    }

    /// Returns true when bootstrap snapshot creation was requested.
    pub fn should_create_snapshot(&self) -> bool {
        self.create_bootstrap_snapshot.is_some()
    }

    /// Creates the configured snapshot archive using a live RocksDB checkpoint as its database source.
    pub fn create_snapshot_from_checkpoint_if_requested(
        &self,
        base_path: &Path,
        chain_config: &Arc<ChainConfig>,
        metadata: BootstrapSnapshotMetadata,
        checkpoint_db: impl FnOnce(&Path) -> anyhow::Result<()>,
    ) -> anyhow::Result<Option<BootstrapSnapshotManifest>> {
        let Some(snapshot_path) = &self.create_bootstrap_snapshot else {
            return Ok(None);
        };
        create_snapshot_from_checkpoint(
            snapshot_path,
            self.create_bootstrap_snapshot_manifest.as_deref(),
            base_path,
            chain_config,
            metadata,
            checkpoint_db,
        )
        .map(Some)
    }
}

/// Manifest for a Madara snapshot archive.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct BootstrapSnapshotManifest {
    /// Snapshot manifest format version. Currently only `1` is supported.
    pub format_version: u32,
    /// Chain id of the snapshot, for example `SN_MAIN`.
    pub chain_id: String,
    /// Latest confirmed block number contained in the archive.
    pub block_number: u64,
    /// Optional latest confirmed block hash at `block_number`.
    #[serde(default)]
    pub block_hash: Option<String>,
    /// Optional global state root at `block_number`.
    #[serde(default)]
    pub state_root: Option<String>,
    /// SHA-256 checksum of the compressed archive, encoded as hex.
    pub archive_sha256: String,
    /// Optional compressed archive size in bytes.
    #[serde(default)]
    pub archive_size_bytes: Option<u64>,
}

impl BootstrapSnapshotManifest {
    /// Parses the expected block hash as a Starknet field element, when present.
    pub fn block_hash_felt(&self) -> anyhow::Result<Option<Felt>> {
        self.block_hash
            .as_deref()
            .map(parse_felt_hex)
            .transpose()
            .with_context(|| "Parsing bootstrap snapshot block_hash")
    }

    /// Parses the expected state root as a Starknet field element, when present.
    pub fn state_root_felt(&self) -> anyhow::Result<Option<Felt>> {
        self.state_root
            .as_deref()
            .map(parse_felt_hex)
            .transpose()
            .with_context(|| "Parsing bootstrap snapshot state_root")
    }
}

/// Chain tip metadata embedded into a bootstrap snapshot manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootstrapSnapshotMetadata {
    /// Latest confirmed block number contained in the snapshot.
    pub block_number: u64,
    /// Latest confirmed block hash at `block_number`.
    pub block_hash: Felt,
    /// Global state root at `block_number`.
    pub state_root: Felt,
}

fn import_snapshot(
    snapshot_path: &Path,
    manifest_path: Option<&Path>,
    base_path: &Path,
    chain_config: &Arc<ChainConfig>,
) -> anyhow::Result<BootstrapSnapshotManifest> {
    let manifest_path = manifest_path.map(Path::to_path_buf).unwrap_or_else(|| default_manifest_path(snapshot_path));
    let manifest = read_manifest(&manifest_path)?;
    validate_snapshot_manifest(&manifest, chain_config)?;
    import_snapshot_archive(snapshot_path, base_path, &manifest)?;

    Ok(manifest)
}

fn validate_snapshot_manifest(
    manifest: &BootstrapSnapshotManifest,
    chain_config: &Arc<ChainConfig>,
) -> anyhow::Result<()> {
    ensure!(
        manifest.format_version == 1,
        "Unsupported bootstrap snapshot manifest format_version={}",
        manifest.format_version
    );
    ensure!(
        manifest.chain_id == chain_config.chain_id.to_string(),
        "Bootstrap snapshot is for chain id `{}`, but this node is configured for `{}`",
        manifest.chain_id,
        chain_config.chain_id
    );
    Ok(())
}

fn import_snapshot_archive(
    snapshot_path: &Path,
    base_path: &Path,
    manifest: &BootstrapSnapshotManifest,
) -> anyhow::Result<()> {
    ensure_base_path_empty(base_path)?;
    verify_archive_metadata(snapshot_path, manifest)?;

    let parent = base_path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).with_context(|| format!("Creating bootstrap snapshot parent {}", parent.display()))?;

    let staging = tempfile::Builder::new()
        .prefix(".madara-bootstrap-")
        .tempdir_in(parent)
        .with_context(|| format!("Creating staging directory in {}", parent.display()))?;
    extract_archive(snapshot_path, staging.path())?;
    validate_extracted_base_path(staging.path())?;

    if base_path.exists() {
        fs::remove_dir(base_path).with_context(|| {
            format!("Removing empty base path {} before installing bootstrap snapshot", base_path.display())
        })?;
    }

    let staging_path = staging.path().to_path_buf();
    fs::rename(&staging_path, base_path).with_context(|| {
        format!("Moving imported bootstrap snapshot from {} to {}", staging_path.display(), base_path.display())
    })?;
    let _ = staging.keep();

    tracing::info!(
        block_number = manifest.block_number,
        chain_id = %manifest.chain_id,
        path = %base_path.display(),
        "Imported bootstrap snapshot"
    );

    Ok(())
}

fn import_remote_snapshot(
    snapshot_url: &Url,
    manifest_url: Option<&Url>,
    base_path: &Path,
    chain_config: &Arc<ChainConfig>,
) -> anyhow::Result<BootstrapSnapshotManifest> {
    let manifest_url = manifest_url.cloned().unwrap_or_else(|| default_manifest_url(snapshot_url));
    let downloads = tempfile::Builder::new()
        .prefix(".madara-bootstrap-download-")
        .tempdir()
        .context("Creating temporary bootstrap snapshot download directory")?;
    let snapshot_path = downloads.path().join("snapshot.tar.gz");
    let manifest_path = downloads.path().join("snapshot.tar.gz.manifest.json");

    download_url_to_file(&manifest_url, &manifest_path)
        .with_context(|| format!("Downloading bootstrap snapshot manifest from {manifest_url}"))?;
    let manifest = read_manifest(&manifest_path)?;
    validate_snapshot_manifest(&manifest, chain_config)?;
    ensure_base_path_empty(base_path)?;

    download_url_to_file(snapshot_url, &snapshot_path)
        .with_context(|| format!("Downloading bootstrap snapshot archive from {snapshot_url}"))?;

    import_snapshot_archive(&snapshot_path, base_path, &manifest)?;
    Ok(manifest)
}

#[cfg(test)]
fn create_snapshot(
    snapshot_path: &Path,
    manifest_path: Option<&Path>,
    base_path: &Path,
    chain_config: &Arc<ChainConfig>,
    snapshot_metadata: BootstrapSnapshotMetadata,
) -> anyhow::Result<BootstrapSnapshotManifest> {
    create_snapshot_from_source(snapshot_path, manifest_path, base_path, base_path, chain_config, snapshot_metadata)
}

fn create_snapshot_from_checkpoint(
    snapshot_path: &Path,
    manifest_path: Option<&Path>,
    base_path: &Path,
    chain_config: &Arc<ChainConfig>,
    snapshot_metadata: BootstrapSnapshotMetadata,
    checkpoint_db: impl FnOnce(&Path) -> anyhow::Result<()>,
) -> anyhow::Result<BootstrapSnapshotManifest> {
    let manifest_path = manifest_path.map(Path::to_path_buf).unwrap_or_else(|| default_manifest_path(snapshot_path));
    ensure_snapshot_output_paths(base_path, snapshot_path, &manifest_path)?;

    let parent = snapshot_path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .with_context(|| format!("Creating bootstrap snapshot checkpoint parent {}", parent.display()))?;
    let staging = tempfile::Builder::new()
        .prefix(".madara-bootstrap-checkpoint-")
        .tempdir_in(parent)
        .with_context(|| format!("Creating bootstrap snapshot checkpoint staging directory in {}", parent.display()))?;

    fs::copy(base_path.join(".db-version"), staging.path().join(".db-version")).with_context(|| {
        format!("Copying {} into bootstrap snapshot checkpoint", base_path.join(".db-version").display())
    })?;
    checkpoint_db(&staging.path().join("db"))?;

    create_snapshot_from_source(
        snapshot_path,
        Some(&manifest_path),
        staging.path(),
        base_path,
        chain_config,
        snapshot_metadata,
    )
}

fn create_snapshot_from_source(
    snapshot_path: &Path,
    manifest_path: Option<&Path>,
    source_base_path: &Path,
    output_guard_base_path: &Path,
    chain_config: &Arc<ChainConfig>,
    snapshot_metadata: BootstrapSnapshotMetadata,
) -> anyhow::Result<BootstrapSnapshotManifest> {
    let manifest_path = manifest_path.map(Path::to_path_buf).unwrap_or_else(|| default_manifest_path(snapshot_path));

    validate_extracted_base_path(source_base_path)?;
    ensure_snapshot_output_paths(output_guard_base_path, snapshot_path, &manifest_path)?;

    write_archive(source_base_path, snapshot_path)?;
    let archive_metadata = fs::metadata(snapshot_path)
        .with_context(|| format!("Reading bootstrap snapshot archive metadata {}", snapshot_path.display()))?;

    let manifest = BootstrapSnapshotManifest {
        format_version: 1,
        chain_id: chain_config.chain_id.to_string(),
        block_number: snapshot_metadata.block_number,
        block_hash: Some(format!("{:#x}", snapshot_metadata.block_hash)),
        state_root: Some(format!("{:#x}", snapshot_metadata.state_root)),
        archive_sha256: sha256_file(snapshot_path)?,
        archive_size_bytes: Some(archive_metadata.len()),
    };

    write_manifest(&manifest_path, &manifest)?;

    tracing::info!(
        block_number = manifest.block_number,
        chain_id = %manifest.chain_id,
        archive_path = %snapshot_path.display(),
        manifest_path = %manifest_path.display(),
        "Created bootstrap snapshot"
    );

    Ok(manifest)
}

fn default_manifest_path(snapshot_path: &Path) -> PathBuf {
    let mut path = snapshot_path.as_os_str().to_os_string();
    path.push(".manifest.json");
    PathBuf::from(path)
}

fn default_manifest_url(snapshot_url: &Url) -> Url {
    let mut manifest_url = snapshot_url.clone();
    let mut path = manifest_url.path().to_owned();
    path.push_str(".manifest.json");
    manifest_url.set_path(&path);
    manifest_url.set_fragment(None);
    manifest_url
}

fn read_manifest(path: &Path) -> anyhow::Result<BootstrapSnapshotManifest> {
    let manifest =
        fs::read_to_string(path).with_context(|| format!("Reading bootstrap snapshot manifest {}", path.display()))?;
    serde_json::from_str(&manifest).with_context(|| format!("Parsing bootstrap snapshot manifest {}", path.display()))
}

fn write_manifest(path: &Path, manifest: &BootstrapSnapshotManifest) -> anyhow::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .with_context(|| format!("Creating bootstrap snapshot manifest parent {}", parent.display()))?;

    let mut tmp = tempfile::Builder::new()
        .prefix(".madara-bootstrap-manifest-")
        .tempfile_in(parent)
        .with_context(|| format!("Creating temporary bootstrap snapshot manifest in {}", parent.display()))?;
    let json = serde_json::to_string_pretty(manifest).context("Serializing bootstrap snapshot manifest")?;
    tmp.write_all(json.as_bytes())
        .with_context(|| format!("Writing temporary bootstrap snapshot manifest {}", tmp.path().display()))?;
    tmp.write_all(b"\n")
        .with_context(|| format!("Writing temporary bootstrap snapshot manifest {}", tmp.path().display()))?;
    tmp.as_file()
        .sync_all()
        .with_context(|| format!("Syncing temporary bootstrap snapshot manifest {}", tmp.path().display()))?;
    tmp.persist_noclobber(path)
        .map(|_| ())
        .with_context(|| format!("Writing bootstrap snapshot manifest {}", path.display()))
}

fn ensure_base_path_empty(path: &Path) -> anyhow::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_dir() => {
            bail!("Bootstrap snapshot base path {} exists and is not a directory", path.display())
        }
        Ok(_) => {
            let mut entries = fs::read_dir(path)
                .with_context(|| format!("Reading bootstrap snapshot base path {}", path.display()))?;
            ensure!(
                entries.next().transpose()?.is_none(),
                "Bootstrap snapshot base path {} is not empty; refusing to overwrite an existing database",
                path.display()
            );
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(err).with_context(|| format!("Inspecting bootstrap snapshot base path {}", path.display()))
        }
    }
    Ok(())
}

fn ensure_snapshot_output_paths(base_path: &Path, archive_path: &Path, manifest_path: &Path) -> anyhow::Result<()> {
    ensure!(!archive_path.exists(), "Bootstrap snapshot archive {} already exists", archive_path.display());
    ensure!(!manifest_path.exists(), "Bootstrap snapshot manifest {} already exists", manifest_path.display());

    let base_path = base_path
        .canonicalize()
        .with_context(|| format!("Canonicalizing bootstrap snapshot base path {}", base_path.display()))?;

    for path in [archive_path, manifest_path] {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)
            .with_context(|| format!("Creating bootstrap snapshot output parent {}", parent.display()))?;
        let parent = parent
            .canonicalize()
            .with_context(|| format!("Canonicalizing bootstrap snapshot output parent {}", parent.display()))?;
        ensure!(
            !parent.starts_with(&base_path),
            "Bootstrap snapshot output {} must be outside source base path {}",
            path.display(),
            base_path.display()
        );
    }

    Ok(())
}

fn verify_archive_metadata(path: &Path, manifest: &BootstrapSnapshotManifest) -> anyhow::Result<()> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("Reading bootstrap snapshot archive metadata {}", path.display()))?;
    ensure!(metadata.is_file(), "Bootstrap snapshot archive {} is not a file", path.display());

    if let Some(expected) = manifest.archive_size_bytes {
        ensure!(
            metadata.len() == expected,
            "Bootstrap snapshot archive size mismatch for {}: expected {} bytes, got {} bytes",
            path.display(),
            expected,
            metadata.len()
        );
    }

    let actual_hash = sha256_file(path)?;
    let expected_hash = normalize_sha256(&manifest.archive_sha256)?;
    ensure!(
        actual_hash == expected_hash,
        "Bootstrap snapshot archive checksum mismatch for {}: expected {}, got {}",
        path.display(),
        expected_hash,
        actual_hash
    );

    Ok(())
}

fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let file = File::open(path).with_context(|| format!("Opening bootstrap snapshot archive {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 128 * 1024];

    loop {
        let read = reader
            .read(&mut buffer)
            .with_context(|| format!("Reading bootstrap snapshot archive {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

fn normalize_sha256(value: &str) -> anyhow::Result<String> {
    let mut normalized = value.trim();
    if let Some(rest) = normalized.strip_prefix("sha256:") {
        normalized = rest;
    }
    if let Some(rest) = normalized.strip_prefix("0x") {
        normalized = rest;
    }
    let normalized = normalized.to_ascii_lowercase();

    ensure!(
        normalized.len() == 64 && normalized.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid bootstrap snapshot archive_sha256 `{value}`; expected 64 hex characters"
    );
    Ok(normalized)
}

fn download_url_to_file(url: &Url, destination: &Path) -> anyhow::Result<()> {
    let url = url.clone();
    let destination = destination.to_path_buf();

    std::thread::spawn(move || download_url_to_file_blocking(&url, &destination))
        .join()
        .map_err(|_| anyhow::anyhow!("Bootstrap snapshot download thread panicked"))?
}

fn download_url_to_file_blocking(url: &Url, destination: &Path) -> anyhow::Result<()> {
    ensure!(
        matches!(url.scheme(), "http" | "https"),
        "Unsupported bootstrap snapshot URL scheme `{}` for {}; expected http or https",
        url.scheme(),
        url
    );

    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .with_context(|| format!("Creating bootstrap snapshot download parent {}", parent.display()))?;

    let mut response = reqwest::blocking::get(url.clone())
        .with_context(|| format!("Requesting bootstrap snapshot URL {url}"))?
        .error_for_status()
        .with_context(|| format!("Bootstrap snapshot URL {url} returned an error status"))?;

    let mut tmp = tempfile::Builder::new()
        .prefix(".madara-bootstrap-download-")
        .tempfile_in(parent)
        .with_context(|| format!("Creating temporary bootstrap snapshot download in {}", parent.display()))?;
    std::io::copy(&mut response, &mut tmp)
        .with_context(|| format!("Writing bootstrap snapshot download for {url} to {}", tmp.path().display()))?;
    tmp.as_file()
        .sync_all()
        .with_context(|| format!("Syncing bootstrap snapshot download {}", tmp.path().display()))?;
    tmp.persist_noclobber(destination)
        .map(|_| ())
        .with_context(|| format!("Writing bootstrap snapshot download {}", destination.display()))
}

fn write_archive(base_path: &Path, archive_path: &Path) -> anyhow::Result<()> {
    let parent = archive_path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .with_context(|| format!("Creating bootstrap snapshot archive parent {}", parent.display()))?;

    let mut tmp = tempfile::Builder::new()
        .prefix(".madara-bootstrap-snapshot-")
        .suffix(".tar.gz")
        .tempfile_in(parent)
        .with_context(|| format!("Creating temporary bootstrap snapshot archive in {}", parent.display()))?;
    let tmp_path = tmp.path().to_path_buf();

    {
        let encoder = GzEncoder::new(&mut tmp, Compression::default());
        let mut archive = tar::Builder::new(encoder);
        append_base_path_contents(&mut archive, base_path)?;
        let encoder = archive
            .into_inner()
            .with_context(|| format!("Finishing bootstrap snapshot archive {}", tmp_path.display()))?;
        encoder
            .finish()
            .with_context(|| format!("Finishing gzip bootstrap snapshot archive {}", tmp_path.display()))?;
    }

    tmp.as_file()
        .sync_all()
        .with_context(|| format!("Syncing temporary bootstrap snapshot archive {}", tmp_path.display()))?;
    tmp.persist_noclobber(archive_path)
        .map(|_| ())
        .with_context(|| format!("Writing bootstrap snapshot archive {}", archive_path.display()))
}

fn append_base_path_contents<W: Write>(archive: &mut tar::Builder<W>, base_path: &Path) -> anyhow::Result<()> {
    append_directory_contents(archive, base_path, base_path)
}

fn append_directory_contents<W: Write>(
    archive: &mut tar::Builder<W>,
    base_path: &Path,
    directory: &Path,
) -> anyhow::Result<()> {
    let mut entries = fs::read_dir(directory)
        .with_context(|| format!("Reading bootstrap snapshot directory {}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("Reading bootstrap snapshot directory entries {}", directory.display()))?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        append_path_to_archive(archive, base_path, &entry.path())?;
    }

    Ok(())
}

fn append_path_to_archive<W: Write>(
    archive: &mut tar::Builder<W>,
    base_path: &Path,
    path: &Path,
) -> anyhow::Result<()> {
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("Reading bootstrap snapshot entry {}", path.display()))?;
    let relative_path = path
        .strip_prefix(base_path)
        .with_context(|| format!("Computing bootstrap snapshot relative path for {}", path.display()))?;

    ensure!(
        metadata.is_file() || metadata.is_dir(),
        "Bootstrap snapshot source entry {} has unsupported file type; only regular files and directories are allowed",
        path.display()
    );

    if metadata.is_dir() {
        archive
            .append_dir(relative_path, path)
            .with_context(|| format!("Adding bootstrap snapshot directory {}", relative_path.display()))?;
        append_directory_contents(archive, base_path, path)?;
    } else {
        archive
            .append_path_with_name(path, relative_path)
            .with_context(|| format!("Adding bootstrap snapshot file {}", relative_path.display()))?;
    }

    Ok(())
}

fn extract_archive(archive_path: &Path, destination: &Path) -> anyhow::Result<()> {
    let file = File::open(archive_path)
        .with_context(|| format!("Opening bootstrap snapshot archive {}", archive_path.display()))?;
    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);

    for entry in archive.entries().context("Reading bootstrap snapshot archive entries")? {
        let mut entry = entry.context("Reading bootstrap snapshot archive entry")?;
        let entry_path = entry.path().context("Reading bootstrap snapshot archive entry path")?.into_owned();
        let entry_type = entry.header().entry_type();

        ensure!(
            entry_type.is_file() || entry_type.is_dir(),
            "Bootstrap snapshot archive entry {} has unsupported type {:?}; only regular files and directories are allowed",
            entry_path.display(),
            entry_type
        );

        let unpacked = entry
            .unpack_in(destination)
            .with_context(|| format!("Unpacking bootstrap snapshot archive into {}", destination.display()))?;
        ensure!(
            unpacked,
            "Bootstrap snapshot archive entry {} is outside destination {}",
            entry_path.display(),
            destination.display()
        );
    }

    Ok(())
}

fn validate_extracted_base_path(path: &Path) -> anyhow::Result<()> {
    ensure!(
        path.join("db").is_dir(),
        "Bootstrap snapshot archive must contain the Madara RocksDB directory `db/` at its root"
    );
    ensure!(path.join(".db-version").is_file(), "Bootstrap snapshot archive must contain `.db-version` at its root");
    Ok(())
}

fn parse_felt_hex(value: &str) -> anyhow::Result<Felt> {
    Felt::from_hex(value).with_context(|| format!("Parsing Starknet felt `{value}`"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::HashMap,
        io::{BufRead, BufReader as IoBufReader},
        net::{TcpListener, TcpStream},
        thread,
    };
    use tar::{Builder, EntryType};
    use tempfile::TempDir;

    #[test]
    fn default_manifest_path_appends_manifest_suffix() {
        assert_eq!(
            default_manifest_path(Path::new("/tmp/mainnet.tar.gz")),
            PathBuf::from("/tmp/mainnet.tar.gz.manifest.json")
        );
    }

    #[test]
    fn default_manifest_url_appends_manifest_suffix_to_path() {
        let url = Url::parse("https://snapshots.example/mainnet.tar.gz?token=abc#ignored").unwrap();
        assert_eq!(
            default_manifest_url(&url).as_str(),
            "https://snapshots.example/mainnet.tar.gz.manifest.json?token=abc"
        );
    }

    #[test]
    fn normalize_sha256_accepts_common_prefixes() {
        let hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert_eq!(normalize_sha256(hash).unwrap(), hash);
        assert_eq!(normalize_sha256(&format!("sha256:{hash}")).unwrap(), hash);
        assert_eq!(normalize_sha256(&format!("0x{hash}")).unwrap(), hash);
        assert_eq!(normalize_sha256(&format!("sha256:0x{hash}")).unwrap(), hash);
    }

    #[test]
    fn ensure_base_path_empty_rejects_non_empty_directory() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("existing"), b"data").unwrap();

        let err = ensure_base_path_empty(temp.path()).unwrap_err();
        assert!(err.to_string().contains("not empty"));
    }

    #[test]
    fn verify_archive_metadata_rejects_bad_hash() {
        let temp = TempDir::new().unwrap();
        let archive_path = temp.path().join("snapshot.tar.gz");
        fs::write(&archive_path, b"not a real archive").unwrap();
        let manifest = BootstrapSnapshotManifest {
            format_version: 1,
            chain_id: "SN_SEPOLIA".to_string(),
            block_number: 0,
            block_hash: None,
            state_root: None,
            archive_sha256: "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            archive_size_bytes: Some(18),
        };

        let err = verify_archive_metadata(&archive_path, &manifest).unwrap_err();
        assert!(err.to_string().contains("checksum mismatch"));
    }

    #[test]
    fn extract_archive_rejects_path_traversal() {
        let temp = TempDir::new().unwrap();
        let archive_path = temp.path().join("snapshot.tar.gz");
        write_tar_gz_unchecked_path(&archive_path, "../escape", b"bad");

        let out = temp.path().join("out");
        fs::create_dir(&out).unwrap();
        let err = extract_archive(&archive_path, &out).unwrap_err();

        assert!(err.to_string().contains("outside destination"));
        assert!(!temp.path().join("escape").exists());
    }

    #[test]
    fn extract_archive_rejects_link_entries() {
        let temp = TempDir::new().unwrap();
        let archive_path = temp.path().join("snapshot.tar.gz");
        write_tar_gz_link(&archive_path, "db/CURRENT", "../outside");

        let out = temp.path().join("out");
        fs::create_dir(&out).unwrap();
        let err = extract_archive(&archive_path, &out).unwrap_err();

        assert!(err.to_string().contains("unsupported type"));
    }

    #[test]
    fn extract_archive_accepts_base_path_contents() {
        let temp = TempDir::new().unwrap();
        let archive_path = temp.path().join("snapshot.tar.gz");
        write_tar_gz(
            &archive_path,
            &[
                (".db-version", b"14".as_slice()),
                ("db/CURRENT", b"MANIFEST-000001".as_slice()),
                ("db/MANIFEST-000001", b"manifest".as_slice()),
            ],
        );

        let out = temp.path().join("out");
        fs::create_dir(&out).unwrap();
        extract_archive(&archive_path, &out).unwrap();
        validate_extracted_base_path(&out).unwrap();
    }

    #[test]
    fn create_snapshot_writes_archive_and_manifest() {
        let temp = TempDir::new().unwrap();
        let base_path = temp.path().join("base");
        fs::create_dir_all(base_path.join("db")).unwrap();
        fs::write(base_path.join(".db-version"), b"14").unwrap();
        fs::write(base_path.join("db/CURRENT"), b"MANIFEST-000001").unwrap();

        let archive_path = temp.path().join("snapshot.tar.gz");
        let chain_config = Arc::new(ChainConfig::madara_test());
        let metadata = BootstrapSnapshotMetadata {
            block_number: 42,
            block_hash: Felt::from_hex_unchecked("0x123"),
            state_root: Felt::from_hex_unchecked("0x456"),
        };

        let manifest = create_snapshot(&archive_path, None, &base_path, &chain_config, metadata).unwrap();
        let manifest_path = default_manifest_path(&archive_path);
        let manifest_from_disk = read_manifest(&manifest_path).unwrap();

        assert_eq!(manifest, manifest_from_disk);
        assert_eq!(manifest.chain_id, "MADARA_TEST");
        assert_eq!(manifest.block_number, 42);
        assert_eq!(manifest.block_hash.as_deref(), Some("0x123"));
        assert_eq!(manifest.state_root.as_deref(), Some("0x456"));
        assert_eq!(manifest.archive_size_bytes, Some(fs::metadata(&archive_path).unwrap().len()));
        assert_eq!(manifest.archive_sha256, sha256_file(&archive_path).unwrap());

        let out = temp.path().join("out");
        fs::create_dir(&out).unwrap();
        extract_archive(&archive_path, &out).unwrap();
        validate_extracted_base_path(&out).unwrap();
        assert_eq!(fs::read(out.join("db/CURRENT")).unwrap(), b"MANIFEST-000001");
    }

    #[test]
    fn import_remote_snapshot_downloads_archive_and_default_manifest() {
        let temp = TempDir::new().unwrap();
        let base_path = temp.path().join("base");
        fs::create_dir_all(base_path.join("db")).unwrap();
        fs::write(base_path.join(".db-version"), b"14").unwrap();
        fs::write(base_path.join("db/CURRENT"), b"MANIFEST-000001").unwrap();

        let archive_path = temp.path().join("snapshot.tar.gz");
        let chain_config = Arc::new(ChainConfig::madara_test());
        let metadata = BootstrapSnapshotMetadata {
            block_number: 42,
            block_hash: Felt::from_hex_unchecked("0x123"),
            state_root: Felt::from_hex_unchecked("0x456"),
        };
        let expected_manifest = create_snapshot(&archive_path, None, &base_path, &chain_config, metadata).unwrap();
        let manifest_path = default_manifest_path(&archive_path);

        let (server_url, server_handle) =
            serve_files(vec![("/snapshot.tar.gz", archive_path), ("/snapshot.tar.gz.manifest.json", manifest_path)]);
        let target_base_path = temp.path().join("target");
        let imported_manifest = import_remote_snapshot(
            &server_url.join("snapshot.tar.gz").unwrap(),
            None,
            &target_base_path,
            &chain_config,
        )
        .unwrap();
        server_handle.join().unwrap();

        assert_eq!(imported_manifest, expected_manifest);
        validate_extracted_base_path(&target_base_path).unwrap();
        assert_eq!(fs::read(target_base_path.join("db/CURRENT")).unwrap(), b"MANIFEST-000001");
    }

    #[test]
    fn create_snapshot_from_checkpoint_archives_checkpoint_db() {
        let temp = TempDir::new().unwrap();
        let base_path = temp.path().join("base");
        fs::create_dir_all(base_path.join("db")).unwrap();
        fs::write(base_path.join(".db-version"), b"14").unwrap();
        fs::write(base_path.join("db/CURRENT"), b"LIVE-DB").unwrap();

        let archive_path = temp.path().join("snapshot.tar.gz");
        let chain_config = Arc::new(ChainConfig::madara_test());
        let metadata = BootstrapSnapshotMetadata {
            block_number: 42,
            block_hash: Felt::from_hex_unchecked("0x123"),
            state_root: Felt::from_hex_unchecked("0x456"),
        };

        create_snapshot_from_checkpoint(
            &archive_path,
            None,
            &base_path,
            &chain_config,
            metadata,
            |checkpoint_db_path| {
                fs::create_dir_all(checkpoint_db_path)?;
                fs::write(checkpoint_db_path.join("CURRENT"), b"CHECKPOINT-DB")?;
                Ok(())
            },
        )
        .unwrap();

        let out = temp.path().join("out");
        fs::create_dir(&out).unwrap();
        extract_archive(&archive_path, &out).unwrap();
        validate_extracted_base_path(&out).unwrap();

        assert_eq!(fs::read(out.join(".db-version")).unwrap(), b"14");
        assert_eq!(fs::read(out.join("db/CURRENT")).unwrap(), b"CHECKPOINT-DB");
    }

    #[test]
    fn create_snapshot_refuses_output_inside_base_path() {
        let temp = TempDir::new().unwrap();
        let base_path = temp.path().join("base");
        fs::create_dir_all(base_path.join("db")).unwrap();
        fs::write(base_path.join(".db-version"), b"14").unwrap();
        let archive_path = base_path.join("snapshot.tar.gz");
        let chain_config = Arc::new(ChainConfig::madara_test());
        let metadata = BootstrapSnapshotMetadata {
            block_number: 42,
            block_hash: Felt::from_hex_unchecked("0x123"),
            state_root: Felt::from_hex_unchecked("0x456"),
        };

        let err = create_snapshot(&archive_path, None, &base_path, &chain_config, metadata).unwrap_err();
        assert!(err.to_string().contains("must be outside source base path"));
    }

    fn write_tar_gz(path: &Path, files: &[(&str, &[u8])]) {
        let file = File::create(path).unwrap();
        let encoder = GzEncoder::new(file, Compression::default());
        let mut builder = Builder::new(encoder);

        for (name, bytes) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, *name, *bytes).unwrap();
        }

        let encoder = builder.into_inner().unwrap();
        encoder.finish().unwrap();
    }

    fn write_tar_gz_link(path: &Path, name: &str, target: &str) {
        let file = File::create(path).unwrap();
        let encoder = GzEncoder::new(file, Compression::default());
        let mut builder = Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        builder.append_link(&mut header, name, target).unwrap();

        let encoder = builder.into_inner().unwrap();
        encoder.finish().unwrap();
    }

    fn write_tar_gz_unchecked_path(path: &Path, name: &str, bytes: &[u8]) {
        let file = File::create(path).unwrap();
        let encoder = GzEncoder::new(file, Compression::default());
        let mut builder = Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
        header.set_entry_type(EntryType::Regular);
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append(&header, bytes).unwrap();

        let encoder = builder.into_inner().unwrap();
        encoder.finish().unwrap();
    }

    fn serve_files(files: Vec<(&'static str, PathBuf)>) -> (Url, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let request_count = files.len();
        let files = files.into_iter().map(|(path, file)| (path.to_string(), file)).collect::<HashMap<_, _>>();

        let handle = thread::spawn(move || {
            for _ in 0..request_count {
                let (stream, _) = listener.accept().unwrap();
                serve_connection(stream, &files);
            }
        });

        (base_url, handle)
    }

    fn serve_connection(mut stream: TcpStream, files: &HashMap<String, PathBuf>) {
        let mut reader = IoBufReader::new(stream.try_clone().unwrap());
        let mut request_line = String::new();
        reader.read_line(&mut request_line).unwrap();
        let path = request_line.split_whitespace().nth(1).unwrap();

        if let Some(file_path) = files.get(path) {
            let body = fs::read(file_path).unwrap();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
            stream.write_all(&body).unwrap();
        } else {
            stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        }
    }
}
