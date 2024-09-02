use std::hash::Hash;
use std::str::FromStr;

lazy_static::lazy_static! {
    pub static ref SUPPORTED_RPC_VERSIONS: Vec<RpcVersion> = vec![
        RpcVersion::RPC_VERSION_0_6_0,
        RpcVersion::RPC_VERSION_0_7_0,
        RpcVersion::RPC_VERSION_0_7_1,
    ];
}

#[derive(Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize, Hash)]
pub struct RpcVersion([u8; 3]);

#[derive(thiserror::Error, Debug, PartialEq)]
pub enum RpcVersionError {
    #[error("Invalid number in version")]
    InvalidNumber(#[from] std::num::ParseIntError),
    #[error("Too many components in version: {0}")]
    TooManyComponents(usize),
}

impl RpcVersion {
    pub const fn new(major: u8, minor: u8, patch: u8) -> Self {
        RpcVersion([major, minor, patch])
    }

    pub fn endpoint_prefix(&self) -> String {
        format!("/rpc/v{}", self)
    }

    pub const RPC_VERSION_0_6_0: RpcVersion = RpcVersion([0, 6, 0]);
    pub const RPC_VERSION_0_7_0: RpcVersion = RpcVersion([0, 7, 0]);
    pub const RPC_VERSION_0_7_1: RpcVersion = RpcVersion([0, 7, 1]);
    pub const RPC_VERSION_LATEST: RpcVersion = Self::RPC_VERSION_0_7_1;
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
        let mut parts = version_str.split('.');

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
        let version = RpcVersion::from_str("0.11").unwrap();
        assert_eq!(version, RpcVersion::new(0, 11, 0));
        assert_eq!(version.to_string(), "0.11.0");
    }

    #[test]
    fn test_rpc_version_string_3() {
        let version = RpcVersion::from_str("0.11.3").unwrap();
        assert_eq!(version, RpcVersion::new(0, 11, 3));
        assert_eq!(version.to_string(), "0.11.3");
    }

    #[test]
    fn test_rpc_version_string_invalid() {
        assert_eq!(RpcVersion::from_str("1.1.1.1.1"), Err(RpcVersionError::TooManyComponents(5)));

        assert!(
            matches!(RpcVersion::from_str("definitely.not.a.version"), Err(RpcVersionError::InvalidNumber(_))),
            "Expected InvalidNumber error"
        );

        assert!(
            matches!(RpcVersion::from_str("0.256.0"), Err(RpcVersionError::InvalidNumber(_))),
            "Expected InvalidNumber error"
        );
    }

    #[test]
    fn test_rpc_version_comparison() {
        let version_1 = RpcVersion::new(1, 2, 3);
        let version_2 = RpcVersion::new(1, 2, 4);
        let version_3 = RpcVersion::new(1, 3, 0);
        let version_4 = RpcVersion::new(2, 0, 0);

        assert!(version_1 < version_2);
        assert!(version_2 < version_3);
        assert!(version_3 < version_4);
    }
}
