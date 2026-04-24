//! Host compatibility metadata for persisted Cairo Native artifacts.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

const SCHEMA_VERSION: u32 = 1;
const NATIVE_CACHE_ABI_VERSION: &str = "madara-cairo-native-v1";

static CURRENT_FINGERPRINT: LazyLock<NativeHostFingerprint> = LazyLock::new(NativeHostFingerprint::detect);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct NativeHostFingerprint {
    pub(crate) schema_version: u32,
    pub(crate) native_cache_abi_version: String,
    pub(crate) arch: String,
    pub(crate) cpu_vendor: String,
    pub(crate) os: String,
}

impl NativeHostFingerprint {
    pub(crate) fn current() -> &'static Self {
        &CURRENT_FINGERPRINT
    }

    fn detect() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            native_cache_abi_version: NATIVE_CACHE_ABI_VERSION.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            cpu_vendor: detect_cpu_vendor(),
            os: std::env::consts::OS.to_string(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum NativeCacheMetadataError {
    #[error("metadata is missing at {path}")]
    Missing { path: PathBuf },
    #[error("metadata is invalid at {path}: {reason}")]
    Invalid { path: PathBuf, reason: String },
    #[error("metadata does not match current host at {path}")]
    Mismatch { path: PathBuf, cached: Box<NativeHostFingerprint>, current: Box<NativeHostFingerprint> },
    #[error("failed to write metadata at {path}: {reason}")]
    Write { path: PathBuf, reason: String },
}

pub(crate) fn metadata_path_for_so(so_path: &Path) -> PathBuf {
    let mut path = so_path.as_os_str().to_os_string();
    path.push(".meta.json");
    PathBuf::from(path)
}

pub(crate) fn validate_metadata_for_so(so_path: &Path) -> Result<NativeHostFingerprint, NativeCacheMetadataError> {
    let cached = read_metadata_for_so(so_path)?;
    let current = NativeHostFingerprint::current();

    if &cached == current {
        Ok(cached)
    } else {
        Err(NativeCacheMetadataError::Mismatch {
            path: metadata_path_for_so(so_path),
            cached: Box::new(cached),
            current: Box::new(current.clone()),
        })
    }
}

pub(crate) fn write_current_metadata_for_so(so_path: &Path) -> Result<(), NativeCacheMetadataError> {
    write_metadata_for_so(so_path, NativeHostFingerprint::current())
}

pub(crate) fn write_metadata_for_so(
    so_path: &Path,
    metadata: &NativeHostFingerprint,
) -> Result<(), NativeCacheMetadataError> {
    let metadata_path = metadata_path_for_so(so_path);
    let tmp_path = tmp_metadata_path(&metadata_path);

    if let Some(parent) = metadata_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| NativeCacheMetadataError::Write { path: metadata_path.clone(), reason: e.to_string() })?;
    }

    let bytes = serde_json::to_vec_pretty(metadata)
        .map_err(|e| NativeCacheMetadataError::Write { path: metadata_path.clone(), reason: e.to_string() })?;

    let write_result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp_path)?;
        file.write_all(&bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp_path, &metadata_path)?;

        if let Some(parent) = metadata_path.parent() {
            let _ = std::fs::File::open(parent).and_then(|dir| dir.sync_all());
        }

        Ok(())
    })();

    write_result.map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        NativeCacheMetadataError::Write { path: metadata_path, reason: e.to_string() }
    })
}

fn read_metadata_for_so(so_path: &Path) -> Result<NativeHostFingerprint, NativeCacheMetadataError> {
    let metadata_path = metadata_path_for_so(so_path);
    let bytes = match std::fs::read(&metadata_path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(NativeCacheMetadataError::Missing { path: metadata_path });
        }
        Err(e) => {
            return Err(NativeCacheMetadataError::Invalid { path: metadata_path, reason: e.to_string() });
        }
    };

    serde_json::from_slice(&bytes)
        .map_err(|e| NativeCacheMetadataError::Invalid { path: metadata_path, reason: e.to_string() })
}

fn tmp_metadata_path(metadata_path: &Path) -> PathBuf {
    let mut path = metadata_path.as_os_str().to_os_string();
    path.push(".tmp");
    PathBuf::from(path)
}

fn detect_cpu_vendor() -> String {
    #[cfg(target_os = "linux")]
    {
        read_linux_cpuinfo_value("vendor_id").unwrap_or_else(|| "unknown".to_string())
    }

    #[cfg(not(target_os = "linux"))]
    {
        "unknown".to_string()
    }
}

#[cfg(target_os = "linux")]
fn read_linux_cpuinfo_value(key: &str) -> Option<String> {
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").ok()?;

    cpuinfo.lines().find_map(|line| {
        let (field, value) = line.split_once(':')?;
        if field.trim() == key {
            let value = value.trim();
            if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            }
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_path_for_so_appends_sidecar_suffix() {
        let so_path = PathBuf::from("/tmp/native_classes/0x1.so");

        assert_eq!(metadata_path_for_so(&so_path), PathBuf::from("/tmp/native_classes/0x1.so.meta.json"));
    }

    #[test]
    fn write_current_metadata_for_so_creates_matching_sidecar() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir should be created");
        let so_path = temp_dir.path().join("0x1.so");
        std::fs::write(&so_path, b"native").expect("test so should be written");

        write_current_metadata_for_so(&so_path).expect("metadata should be written");

        let cached = validate_metadata_for_so(&so_path).expect("metadata should match current host");
        assert_eq!(&cached, NativeHostFingerprint::current());
    }

    #[test]
    fn validate_metadata_for_so_returns_missing_when_sidecar_absent() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir should be created");
        let so_path = temp_dir.path().join("0x1.so");
        std::fs::write(&so_path, b"native").expect("test so should be written");

        let error = validate_metadata_for_so(&so_path).expect_err("metadata should be missing");

        assert!(matches!(error, NativeCacheMetadataError::Missing { .. }));
    }

    #[test]
    fn validate_metadata_for_so_returns_invalid_when_sidecar_is_malformed() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir should be created");
        let so_path = temp_dir.path().join("0x1.so");
        std::fs::write(&so_path, b"native").expect("test so should be written");
        std::fs::write(metadata_path_for_so(&so_path), b"{not-json").expect("metadata should be written");

        let error = validate_metadata_for_so(&so_path).expect_err("metadata should be invalid");

        assert!(matches!(error, NativeCacheMetadataError::Invalid { .. }));
    }

    #[test]
    fn validate_metadata_for_so_returns_mismatch_when_arch_differs() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir should be created");
        let so_path = temp_dir.path().join("0x1.so");
        std::fs::write(&so_path, b"native").expect("test so should be written");

        let mut cached = NativeHostFingerprint::current().clone();
        cached.arch = "different-arch".to_string();
        write_metadata_for_so(&so_path, &cached).expect("metadata should be written");

        let error = validate_metadata_for_so(&so_path).expect_err("metadata should mismatch");

        assert!(matches!(error, NativeCacheMetadataError::Mismatch { .. }));
    }
}
