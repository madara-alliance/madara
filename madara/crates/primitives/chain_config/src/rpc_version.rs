use std::hash::Hash;
use std::str::FromStr;

const SUPPORTED_RPC_VERSIONS: [RpcVersion; 4] = [
    RpcVersion::RPC_VERSION_0_7_1,
    RpcVersion::RPC_VERSION_0_8_1,
    RpcVersion::RPC_VERSION_0_9_0,
    RpcVersion::RPC_VERSION_ADMIN_0_1_0,
];

#[derive(Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize, Hash)]
pub struct RpcVersion([u8; 3]);

#[derive(thiserror::Error, Debug, PartialEq)]
pub enum RpcVersionError {
    #[error("Invalid number in version")]
    InvalidNumber(#[from] std::num::ParseIntError),
    #[error("Too many components in version: {0}")]
    TooManyComponents(usize),
    #[error("Invalid request path specified, could not extract a version")]
    InvalidPathSupplied,
    #[error("Invalid version specified")]
    InvalidVersion,
    #[error("Unsupported version specified")]
    UnsupportedVersion,
}

impl RpcVersion {
    pub const fn new(major: u8, minor: u8, patch: u8) -> Self {
        RpcVersion([major, minor, patch])
    }

    #[tracing::instrument(skip(path), fields(module = "RpcVersion"))]
    pub fn from_request_path(path: &str, version_default: RpcVersion) -> Result<Self, RpcVersionError> {
        tracing::debug!(target: "rpc_version", "extracting rpc version from request: {path}");

        let path = path.to_ascii_lowercase();
        let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();

        tracing::debug!(target: "rpc_version", "version parts are: {parts:?}");

        match parts.as_slice() {
            // Match empty path
            [] => {
                tracing::debug!(target: "rpc_version", "No version specified, defaulting to latest.");
                Ok(version_default)
            }
            // Match valid path format "/rpc/vX_Y_Z"
            ["rpc", version_str] if version_str.starts_with('v') => {
                let version_str = &version_str[1..]; // strip "v"
                match RpcVersion::from_str(version_str) {
                    Ok(version) if SUPPORTED_RPC_VERSIONS.contains(&version) => {
                        tracing::debug!(target: "rpc_version", "Found supported version: {version}");
                        Ok(version)
                    }
                    Ok(_) => {
                        tracing::debug!(target: "rpc_version", "Version unsupported");
                        Err(RpcVersionError::UnsupportedVersion)
                    }
                    Err(_) => {
                        tracing::debug!(target: "rpc_version", "Invalid version format: {version_str}");
                        Err(RpcVersionError::InvalidVersion)
                    }
                }
            }
            // Fallback for invalid format
            _ => {
                tracing::debug!(target: "rpc_version", "Invalid path format, defaulting to latest.");
                Ok(version_default)
            }
        }
    }

    pub fn endpoint_prefix(&self) -> String {
        format!("/rpc/v{}", self)
    }

    pub fn module(&self) -> String {
        format!("v{}_{}_{}", self.0[0], self.0[1], self.0[2])
    }

    pub fn name(&self) -> String {
        format!("V{}_{}_{}", self.0[0], self.0[1], self.0[2])
    }

    pub const RPC_VERSION_0_7_1: RpcVersion = RpcVersion([0, 7, 1]);
    pub const RPC_VERSION_0_8_1: RpcVersion = RpcVersion([0, 8, 1]);
    pub const RPC_VERSION_0_9_0: RpcVersion = RpcVersion([0, 9, 0]);
    pub const RPC_VERSION_LATEST: RpcVersion = Self::RPC_VERSION_0_9_0;

    pub const RPC_VERSION_ADMIN_0_1_0: RpcVersion = RpcVersion([0, 1, 0]);
    pub const RPC_VERSION_LATEST_ADMIN: RpcVersion = Self::RPC_VERSION_ADMIN_0_1_0;
}

impl std::fmt::Display for RpcVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0[0], self.0[1], self.0[2])?;
        Ok(())
    }
}

// fallback to Display
impl std::fmt::Debug for RpcVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl FromStr for RpcVersion {
    type Err = RpcVersionError;

    fn from_str(version_str: &str) -> Result<Self, Self::Err> {
        let mut parts = version_str.split('_');

        let mut version = [0u8; 3];
        for (i, part) in parts.by_ref().take(3).enumerate() {
            version[i] = part.parse()?;
        }
        let extra = parts.count(); // remaining items in the iter
        if extra > 0 {
            return Err(RpcVersionError::TooManyComponents(extra + 3));
        }

        Ok(RpcVersion(version))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rpc_version_string_2() {
        let version = RpcVersion::from_str("0_11").unwrap();
        assert_eq!(version, RpcVersion::new(0, 11, 0));
        assert_eq!(version.to_string(), "0.11.0");
    }

    #[test]
    fn test_rpc_version_string_3() {
        let version = RpcVersion::from_str("0_11_3").unwrap();
        assert_eq!(version, RpcVersion::new(0, 11, 3));
        assert_eq!(version.to_string(), "0.11.3");
    }

    #[test]
    fn test_rpc_version_string_invalid() {
        assert_eq!(RpcVersion::from_str("1_1_1_1_1"), Err(RpcVersionError::TooManyComponents(5)));

        assert!(
            matches!(RpcVersion::from_str("definitely_not_a_version"), Err(RpcVersionError::InvalidNumber(_))),
            "Expected InvalidNumber error"
        );

        assert!(
            matches!(RpcVersion::from_str("0_256_0"), Err(RpcVersionError::InvalidNumber(_))),
            "Expected InvalidNumber error"
        );
    }

    #[test]
    fn test_rpc_version_comparison() {
        let version_1 = RpcVersion::new(1, 2, 3);
        let version_2 = RpcVersion::new(1, 2, 4);
        let version_3 = RpcVersion::new(1, 3, 0);
        let version_4 = RpcVersion::new(2, 0, 0);
        let version_5 = RpcVersion::new(2, 0, 0);

        assert!(version_1 < version_2);
        assert!(version_2 < version_3);
        assert!(version_3 < version_4);
        assert!(version_4 == version_5);
    }

    #[test]
    fn test_from_request_path_valid() {
        assert_eq!(
            RpcVersion::from_request_path("/rpc/v0_7_1/", RpcVersion::RPC_VERSION_LATEST).unwrap(),
            RpcVersion::RPC_VERSION_0_7_1
        );
        assert_eq!(
            RpcVersion::from_request_path("/rpc/v0_7_1", RpcVersion::RPC_VERSION_LATEST).unwrap(),
            RpcVersion::RPC_VERSION_0_7_1
        );
        assert_eq!(
            RpcVersion::from_request_path("/rpc/v0_8_1/", RpcVersion::RPC_VERSION_LATEST).unwrap(),
            RpcVersion::RPC_VERSION_0_8_1
        );
        assert_eq!(
            RpcVersion::from_request_path("/rpc/v0_8_1", RpcVersion::RPC_VERSION_LATEST).unwrap(),
            RpcVersion::RPC_VERSION_0_8_1
        );
    }

    #[test]
    fn test_from_request_path_empty() {
        assert_eq!(
            RpcVersion::from_request_path("", RpcVersion::RPC_VERSION_LATEST).unwrap(),
            RpcVersion::RPC_VERSION_LATEST
        );
    }

    #[test]
    fn test_from_request_path_root() {
        assert_eq!(
            RpcVersion::from_request_path("/", RpcVersion::RPC_VERSION_LATEST).unwrap(),
            RpcVersion::RPC_VERSION_LATEST
        );
    }

    #[test]
    fn test_from_request_path_invalid_format() {
        assert_eq!(
            RpcVersion::from_request_path("/invalid/path", RpcVersion::RPC_VERSION_LATEST).unwrap(),
            RpcVersion::RPC_VERSION_LATEST
        );
    }

    #[test]
    fn test_from_request_path_unsupported_version() {
        assert_eq!(
            RpcVersion::from_request_path("/rpc/v9_9_9", RpcVersion::RPC_VERSION_LATEST),
            Err(RpcVersionError::UnsupportedVersion)
        );
    }

    #[test]
    fn test_from_request_path_invalid_version() {
        assert_eq!(
            RpcVersion::from_request_path("/rpc/vx_y_z", RpcVersion::RPC_VERSION_LATEST),
            Err(RpcVersionError::InvalidVersion)
        );
    }
}
