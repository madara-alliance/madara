use std::str::FromStr;

/// Represents the version of the starknet protocol using a four-component version number
/// (major, minor, patch, build).
///
/// # Current implementation
/// The protocol version field is absent from early blocks on the mainnet.
/// To handle these legacy cases, `StarknetVersion` uses specific predefined versions for
/// blocks where the actual protocol version is unknown. These versions are created to fill
/// in the gaps and enable consistent version-based logic for determining protocol behavior
/// without relying on block numbers or chain IDs.
///
/// These fake protocol versions are tailored for mainnet usage and should not be applied
/// outside of this context. The mapping between mainnet block numbers and `StarknetVersion`
/// is defined to ensure that legacy special cases are handled appropriately based on version.
///
/// # Example
/// ```rust
/// use mp_chain_config::StarknetVersion;
///
/// let version = StarknetVersion::new(0, 9, 1, 0);
/// assert!(!version.is_legacy()); // Checks if the version is considered legacy.
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct StarknetVersion([u8; 4]);

impl Default for StarknetVersion {
    /// Returns the latest version as the default.
    fn default() -> Self {
        StarknetVersion::LATEST
    }
}

#[derive(thiserror::Error, Debug, PartialEq)]
pub enum StarknetVersionError {
    /// Error indicating an invalid number format in the version components.
    #[error("Invalid number in version")]
    InvalidNumber(#[from] std::num::ParseIntError),
    /// Error indicating too many components in the version string, expected at most 4.
    #[error("Too many components in version: expected at most 4 but found {0}")]
    TooManyComponents(usize),
}

impl StarknetVersion {
    /// Creates a new `StarknetVersion` with the specified major, minor, patch, and build components.
    pub const fn new(major: u8, minor: u8, patch: u8, build: u8) -> Self {
        StarknetVersion([major, minor, patch, build])
    }

    pub const V_0_0_0: StarknetVersion = StarknetVersion([0, 0, 0, 0]);
    pub const V0_7_0: StarknetVersion = StarknetVersion([0, 7, 0, 0]);
    pub const FIRST_BLOCK_POST_LEGACY: StarknetVersion = StarknetVersion([0, 7, 1, 0]); // block betwe
    pub const POST_LEGACY: StarknetVersion = StarknetVersion([0, 8, 0, 0]); // TODO: update this when we have the exact version
    pub const DECLARED_CLASS_IN_STATE_UPDATE: StarknetVersion = StarknetVersion([0, 8, 1, 0]); // TODO: update this when we have the exact version
    pub const V0_9_1: StarknetVersion = StarknetVersion([0, 9, 1, 0]); // block 3799 on MAINNET, from this block version is visible
    pub const V0_11_1: StarknetVersion = StarknetVersion([0, 11, 1, 0]);
    pub const V0_13_0: StarknetVersion = StarknetVersion([0, 13, 0, 0]);
    pub const V0_13_1: StarknetVersion = StarknetVersion([0, 13, 1, 0]);
    pub const V0_13_1_1: StarknetVersion = StarknetVersion([0, 13, 1, 1]);
    pub const V0_13_2: StarknetVersion = StarknetVersion([0, 13, 2, 0]);
    pub const V0_13_2_1: StarknetVersion = StarknetVersion([0, 13, 2, 1]);
    pub const V0_13_3: StarknetVersion = StarknetVersion([0, 13, 3, 0]);
    pub const V0_13_4: StarknetVersion = StarknetVersion([0, 13, 4, 0]);
    pub const V0_13_5: StarknetVersion = StarknetVersion([0, 13, 5, 0]);
    pub const V0_14_0: StarknetVersion = StarknetVersion([0, 14, 0, 0]);
    pub const V0_14_1: StarknetVersion = StarknetVersion([0, 14, 1, 0]);
    /// The latest version supported by orchestrator.
    pub const LATEST: StarknetVersion = Self::V0_14_0;

    pub fn is_pre_v0_7(&self) -> bool {
        *self < Self::V0_7_0
    }

    pub fn is_tx_hash_inconsistent(&self) -> bool {
        *self == Self::FIRST_BLOCK_POST_LEGACY
    }

    pub fn is_legacy(&self) -> bool {
        *self < Self::POST_LEGACY
    }

    /// Checks if the version indicates that declared classes are included in state updates.
    pub fn is_declared_class_in_state_update(&self) -> bool {
        *self < Self::DECLARED_CLASS_IN_STATE_UPDATE
    }

    /// Returns true if this version uses BLAKE compiled_class_hash (SNIP-34).
    /// Starting from v0.14.1, newly declared classes use BLAKE hash instead of Poseidon.
    pub fn uses_blake_compiled_class_hash(&self) -> bool {
        *self >= Self::V0_14_1
    }

    /// Attempts to derive a `StarknetVersion` from a mainnet block number.
    ///
    /// # Note
    /// This function is only applicable to mainnet block numbers before the version field was introduced.
    pub fn try_from_mainnet_block_number(block_number: u64) -> Option<StarknetVersion> {
        match block_number {
            0..=832 => Some(Self::V_0_0_0),
            833..=1468 => Some(Self::V0_7_0),
            1469..=1469 => Some(Self::FIRST_BLOCK_POST_LEGACY),
            1470..=2596 => Some(Self::POST_LEGACY),
            2597..=3798 => Some(Self::DECLARED_CLASS_IN_STATE_UPDATE),
            _ => None,
        }
    }
}

impl std::fmt::Display for StarknetVersion {
    /// Formats the `StarknetVersion` as a string in the format `major.minor.patch[.build]`.
    /// The build component is omitted if it is zero.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0[0], self.0[1], self.0[2])?;
        if self.0[3] != 0 {
            write!(f, ".{}", self.0[3])?;
        }
        Ok(())
    }
}

// Fallback to Display implementation for Debug formatting.
impl std::fmt::Debug for StarknetVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl FromStr for StarknetVersion {
    type Err = StarknetVersionError;

    /// Parses a version string in the format `major.minor.patch[.build]` into a `StarknetVersion`.
    ///
    /// Expects up to four components. If more are provided, returns `TooManyComponents` error.
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
