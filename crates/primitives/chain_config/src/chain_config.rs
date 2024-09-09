use crate::StarknetVersion;
use anyhow::{Context, Result};
use blockifier::{
    bouncer::BouncerConfig,
    versioned_constants::VersionedConstants,
};
use primitive_types::H160;
use serde::Deserialize;
use serde::Deserializer;
use starknet_api::core::{ChainId, ContractAddress};
use std::str::FromStr;
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(thiserror::Error, Debug)]
#[error("Unsupported protocol version: {0}")]
pub struct UnsupportedProtocolVersion(StarknetVersion);

#[derive(Debug)]
pub struct ChainVersionedConstants(pub BTreeMap<StarknetVersion, VersionedConstants>);

impl<const N: usize> From<[(StarknetVersion, VersionedConstants); N]> for ChainVersionedConstants {
    fn from(arr: [(StarknetVersion, VersionedConstants); N]) -> Self {
        ChainVersionedConstants(arr.into_iter().collect())
    }
}

/// Replaces the versioned_constants files definition in the yaml by the content of the
/// jsons.
impl<'de> Deserialize<'de> for ChainVersionedConstants {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let file_paths: BTreeMap<String, String> = Deserialize::deserialize(deserializer)?;
        let mut result = BTreeMap::new();

        for (version, path) in file_paths {
            let mut file = File::open(Path::new(&path))
                .with_context(|| format!("Failed to open file: {}", path))
                .map_err(serde::de::Error::custom)?;

            let mut contents = String::new();
            file.read_to_string(&mut contents)
                .with_context(|| format!("Failed to read contents of file: {}", path))
                .map_err(serde::de::Error::custom)?;

            let constants: VersionedConstants = serde_json::from_str(&contents)
                .with_context(|| format!("Failed to parse JSON in file: {}", path))
                .map_err(serde::de::Error::custom)?;

            let parsed_version = version
                .parse()
                .with_context(|| format!("Failed to parse version string: {}", version))
                .map_err(serde::de::Error::custom)?;

            result.insert(parsed_version, constants);
        }

        Ok(ChainVersionedConstants(result))
    }
}

#[derive(Debug, Deserialize)]
pub struct ChainConfig {
    /// Internal chain name.
    pub chain_name: String,
    pub chain_id: ChainId,

    /// For starknet, this is the STRK ERC-20 contract on starknet.
    pub native_fee_token_address: ContractAddress,
    /// For starknet, this is the ETH ERC-20 contract on starknet.
    pub parent_fee_token_address: ContractAddress,

    /// BTreeMap ensures order.
    pub versioned_constants: ChainVersionedConstants,
    pub latest_protocol_version: StarknetVersion,

    /// Only used for block production.
    pub block_time: Duration,
    /// Only used for block production.
    /// Block time is divided into "ticks": everytime this duration elapses, the pending block is updated.  
    pub pending_block_update_time: Duration,

    /// The bouncer is in charge of limiting block sizes. This is where the max number of step per block, gas etc are.
    /// Only used for block production.
    pub bouncer_config: BouncerConfig,

    /// Only used for block production.
    pub sequencer_address: ContractAddress,

    /// Only used when mempool is enabled.
    /// When deploying an account and invoking a contract at the same time, we want to skip the validation step for the invoke tx.
    /// This number is the maximum nonce the invoke tx can have to qualify for the validation skip.
    pub max_nonce_for_validation_skip: u64,

    /// The Starknet core contract address for the L1 watcher.
    pub eth_core_contract_address: H160,
}

impl Default for ChainConfig {
    fn default() -> Self {
        ChainConfig::starknet_mainnet().expect("Invalid preset configuration for mainnet.")
    }
}

impl ChainConfig {
    pub fn from_yaml(path: &Path) -> anyhow::Result<Self> {
        let config_str = fs::read_to_string(path)?;
        serde_yaml::from_str(&config_str).context("While deserializing chain config")
    }

    /// Returns the Chain Config preset for Starknet Mainnet.
    pub fn starknet_mainnet() -> anyhow::Result<Self> {
        // Sources:
        // - https://docs.starknet.io/tools/important-addresses
        // - https://docs.starknet.io/tools/limits-and-triggers (bouncer & block times)
        // - state_diff_size is the blob size limit of ethereum
        // - pending_block_update_time: educated guess
        // - bouncer builtin_count, message_segment_length, n_events, state_diff_size are probably wrong
        Self::from_yaml(&PathBuf::from_str("../presets/mainnet.yaml")?)
    }

    /// Returns the Chain Config preset for Starknet Sepolia.
    pub fn starknet_sepolia() -> anyhow::Result<Self> {
        Self::from_yaml(&PathBuf::from_str("../presets/sepolia.yaml")?)
    }

    /// Returns the Chain Config preset for Starknet Integration.
    pub fn starknet_integration() -> anyhow::Result<Self> {
        Self::from_yaml(&PathBuf::from_str("../presets/integration.yaml")?)
    }

    /// Returns the Chain Config preset for our Madara tests.
    #[cfg(test)]
    pub fn test_config() -> anyhow::Result<Self> {
        Self::from_yaml(&PathBuf::from_str("../presets/test.yaml")?)
    }

    /// This is the number of pending ticks (see [`ChainConfig::pending_block_update_time`]) in a block.
    pub fn n_pending_ticks_per_block(&self) -> usize {
        (self.block_time.as_millis() / self.pending_block_update_time.as_millis()) as usize
    }

    pub fn exec_constants_by_protocol_version(
        &self,
        version: StarknetVersion,
    ) -> Result<VersionedConstants, UnsupportedProtocolVersion> {
        for (k, constants) in self.versioned_constants.0.iter().rev() {
            if k <= &version {
                return Ok(constants.clone());
            }
        }
        Err(UnsupportedProtocolVersion(version))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exec_constants() {
        let chain_config = ChainConfig {
            versioned_constants: [
                (StarknetVersion::new(0, 1, 5, 0), {
                    let mut constants = VersionedConstants::default();
                    constants.validate_max_n_steps = 5;
                    constants
                }),
                (StarknetVersion::new(0, 2, 0, 0), {
                    let mut constants = VersionedConstants::default();
                    constants.validate_max_n_steps = 10;
                    constants
                }),
            ].into(),
            ..ChainConfig::test_config().unwrap()
        };

        assert_eq!(
            chain_config
                .exec_constants_by_protocol_version(StarknetVersion::new(0, 1, 5, 0))
                .unwrap()
                .validate_max_n_steps,
            5
        );
        assert_eq!(
            chain_config
                .exec_constants_by_protocol_version(StarknetVersion::new(0, 1, 6, 0))
                .unwrap()
                .validate_max_n_steps,
            5
        );
        assert_eq!(
            chain_config
                .exec_constants_by_protocol_version(StarknetVersion::new(0, 1, 7, 0))
                .unwrap()
                .validate_max_n_steps,
            5
        );
        assert_eq!(
            chain_config
                .exec_constants_by_protocol_version(StarknetVersion::new(0, 2, 0, 0))
                .unwrap()
                .validate_max_n_steps,
            10
        );
        assert_eq!(
            chain_config
                .exec_constants_by_protocol_version(StarknetVersion::new(0, 2, 5, 0))
                .unwrap()
                .validate_max_n_steps,
            10
        );
        assert_eq!(
            chain_config
                .exec_constants_by_protocol_version(StarknetVersion::new(1, 0, 0, 0))
                .unwrap()
                .validate_max_n_steps,
            10
        );
        assert!(chain_config.exec_constants_by_protocol_version(StarknetVersion::new(0, 0, 0, 0)).is_err(),);
    }
}
