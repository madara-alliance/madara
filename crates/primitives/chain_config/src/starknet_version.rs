use std::str::FromStr;

#[derive(Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct StarknetVersion([u8; 4]);

#[derive(thiserror::Error, Debug, PartialEq)]
pub enum StarknetVersionError {
    #[error("Invalid number in version")]
    InvalidNumber(#[from] std::num::ParseIntError),
    #[error("Too many components in version: {0}")]
    TooManyComponents(usize),
}

impl StarknetVersion {
    pub const fn new(major: u8, minor: u8, patch: u8, build: u8) -> Self {
        StarknetVersion([major, minor, patch, build])
    }

    pub const STARKNET_VERSION_0_11_1: StarknetVersion = StarknetVersion([0, 11, 1, 0]);
    pub const STARKNET_VERSION_0_13_0: StarknetVersion = StarknetVersion([0, 13, 0, 0]);
    pub const STARKNET_VERSION_0_13_1: StarknetVersion = StarknetVersion([0, 13, 1, 0]);
    pub const STARKNET_VERSION_0_13_1_1: StarknetVersion = StarknetVersion([0, 13, 1, 1]);
    pub const STARKNET_VERSION_0_13_2: StarknetVersion = StarknetVersion([0, 13, 2, 0]);
    pub const STARKNET_VERSION_LATEST: StarknetVersion = Self::STARKNET_VERSION_0_13_2;
}

impl std::fmt::Display for StarknetVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0[0], self.0[1], self.0[2])?;
        if self.0[3] != 0 {
            write!(f, ".{}", self.0[3])?;
        }
        Ok(())
    }
}

// fallback to Display
impl std::fmt::Debug for StarknetVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl FromStr for StarknetVersion {
    type Err = StarknetVersionError;

    fn from_str(version_str: &str) -> Result<Self, Self::Err> {
        let mut parts = version_str.split('.');

        let mut version = [0u8; 4];
        for (i, part) in parts.by_ref().take(4).enumerate() {
            version[i] = part.parse()?;
        }
        let extra = parts.count(); // remaining items in the iter
        if extra > 0 {
            return Err(StarknetVersionError::TooManyComponents(extra + 4));
        }

        Ok(StarknetVersion(version))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_starknet_version_string_3() {
        let version = StarknetVersion::from_str("0.11.3").unwrap();
        assert_eq!(version, StarknetVersion::new(0, 11, 3, 0));
        assert_eq!(version.to_string(), "0.11.3");
    }

    #[test]
    fn test_starknet_version_string_4() {
        let version = StarknetVersion::from_str("1.2.3.4").unwrap();
        assert_eq!(version, StarknetVersion::new(1, 2, 3, 4));
        assert_eq!(version.to_string(), "1.2.3.4");
    }

    #[test]
    fn test_starknet_version_string_invalid() {
        assert_eq!(StarknetVersion::from_str("1.1.1.1.1"), Err(StarknetVersionError::TooManyComponents(5)));

        assert!(
            matches!(
                StarknetVersion::from_str("definitely.not.a.version"),
                Err(StarknetVersionError::InvalidNumber(_))
            ),
            "Expected InvalidNumber error"
        );

        assert!(
            matches!(StarknetVersion::from_str("0.256.0"), Err(StarknetVersionError::InvalidNumber(_))),
            "Expected InvalidNumber error"
        );
    }

    #[test]
    fn test_starknet_version_comparison() {
        let version_1 = StarknetVersion::new(1, 2, 3, 4);
        let version_2 = StarknetVersion::new(1, 2, 3, 5);
        let version_3 = StarknetVersion::new(1, 2, 4, 0);
        let version_4 = StarknetVersion::new(1, 3, 0, 0);
        let version_5 = StarknetVersion::new(2, 0, 0, 0);

        assert!(version_1 < version_2);
        assert!(version_2 < version_3);
        assert!(version_3 < version_4);
        assert!(version_4 < version_5);
    }
}
