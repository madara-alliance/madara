//! Runtime execution configuration for block production.
//!
//! Provides types and serialization logic to persist execution configuration when starting a new block.
//! This ensures consistent re-execution on node restarts, even if the configuration changes.
//!
//! ## Version Handling
//!
//! ChainConfig versioning is handled automatically via round-trip serialization:
//! 1. **Serialize**: Config is saved with its version tag.
//! 2. **Deserialize**: `ChainConfigVersioned` handles version migrations automatically.
//! 3. **Result**: Re-execution always uses the original config values, regardless of current node version.
//!
//! ### Note on Versioned Constants
//! `versioned_constants` (gas costs, limits) are NOT stored in the database as they are not serializable.
//! Instead, we store the `protocol_version` and reconstruct the constants from the node's built-in definitions
//! upon restart. This assumes protocol constants for a given version are immutable.
//!
//! ### Maintenance
//! When adding a new `ChainConfig` version, update `ChainConfigVersioned` enum and `TryFrom` implementations.
//! No changes are needed in this module.

use crate::{ChainConfig, ChainConfigVersioned, ChainVersionedConstants, StarknetVersion};
use anyhow::{Context, Result};
use blockifier::blockifier_versioned_constants::VersionedConstants;
use mp_utils::crypto::ZeroingPrivateKey;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Saved runtime config ensures re-execution consistency on restart.
#[derive(Debug)]
pub struct RuntimeExecutionConfig {
    /// Chain config used during execution
    pub chain_config: ChainConfig,
    /// Resolved execution constants
    pub exec_constants: VersionedConstants,
    /// Fee charging flag
    pub no_charge_fee: bool,
}

/// Internal serializable form for database storage.
#[derive(Serialize, Deserialize)]
pub struct RuntimeExecutionConfigSerializable {
    /// Config version for tracking/migration
    config_version: u32,
    /// YAML-serialized chain config
    chain_config_json: String,
    /// Protocol version string
    protocol_version: String,
    /// Fee charging flag
    no_charge_fee: bool,
}

impl RuntimeExecutionConfig {
    /// Copies ChainConfig via version-aware round-trip serialization.
    fn copy_chain_config_via_serialization(config: &ChainConfig) -> Result<ChainConfig> {
        use serde_yaml;

        // Serialize ChainConfig to versioned YAML format
        let yaml_str = Self::chain_config_to_versioned_yaml(config)?;

        // Deserialize back using ChainConfigVersioned (handles all versions automatically)
        let versioned: ChainConfigVersioned =
            serde_yaml::from_str(&yaml_str).context("Failed to deserialize ChainConfig from YAML during copy")?;

        // Convert to canonical ChainConfig
        ChainConfig::try_from(versioned)
            .context("Failed to convert versioned ChainConfig to canonical ChainConfig during copy")
    }

    /// Converts ChainConfig to versioned YAML string.
    fn chain_config_to_versioned_yaml(config: &ChainConfig) -> Result<String> {
        serde_yaml::to_string(config).context("Failed to serialize ChainConfig")
    }

    /// Creates config from Arc<ChainConfig> using version-aware copying.
    pub fn from_arc_chain_config(
        chain_config: &Arc<ChainConfig>,
        exec_constants: VersionedConstants,
        no_charge_fee: bool,
    ) -> Result<Self> {
        // Copy via serialization for version handling
        let mut chain_config_copy = Self::copy_chain_config_via_serialization(chain_config)
            .context("Failed to copy ChainConfig via serialization")?;

        // Copy versioned_constants manually
        let mut versioned_constants_copy = ChainVersionedConstants::default();
        for (k, v) in &chain_config.versioned_constants.0 {
            versioned_constants_copy.add(*k, v.clone());
        }
        chain_config_copy.versioned_constants = versioned_constants_copy;

        // Use default private_key since it's not serialized and ZeroingPrivateKey doesn't implement Clone
        // This is safe because:
        // 1. The copied ChainConfig is only used for re-execution (which doesn't require private_key)
        // 2. When closing the block, the backend's CURRENT chain_config is used for computing block hash and commitments
        // 3. When blocks are signed (on-demand via gateway API), the backend's CURRENT chain_config.private_key is used
        // Therefore, using default() here is acceptable since private_key is never accessed from the copied config

        Ok(Self { chain_config: chain_config_copy, exec_constants, no_charge_fee })
    }

    /// Converts to serializable form for DB storage.
    pub fn to_serializable(&self) -> Result<RuntimeExecutionConfigSerializable> {
        // Convert ChainConfig to versioned YAML string
        let chain_config_yaml = Self::chain_config_to_versioned_yaml(&self.chain_config)?;

        // Store protocol version (reconstruct VersionedConstants on load)
        let protocol_version_str = self.chain_config.latest_protocol_version.to_string();

        Ok(RuntimeExecutionConfigSerializable {
            config_version: self.chain_config.config_version,
            chain_config_json: chain_config_yaml,
            protocol_version: protocol_version_str,
            no_charge_fee: self.no_charge_fee,
        })
    }

    /// Reconstructs from serializable form.
    pub fn from_serializable(
        serializable: RuntimeExecutionConfigSerializable,
        backend_chain_config: &ChainConfig,
    ) -> Result<Self> {
        use serde_yaml;

        if serializable.config_version != backend_chain_config.config_version {
            tracing::warn!(
                "RuntimeExecutionConfig version mismatch: saved {} != current {}. Using saved config.",
                serializable.config_version,
                backend_chain_config.config_version
            );
        }

        // Deserialize ChainConfig from YAML (handles versioning)
        let versioned: ChainConfigVersioned = serde_yaml::from_str(&serializable.chain_config_json)
            .context("Failed to deserialize ChainConfig from YAML")?;

        let mut chain_config = ChainConfig::try_from(versioned)
            .context("Failed to convert versioned ChainConfig to canonical ChainConfig")?;

        // Merge versioned_constants from backend
        chain_config.versioned_constants = {
            let mut vc = ChainVersionedConstants::default();
            for (k, v) in &backend_chain_config.versioned_constants.0 {
                vc.add(*k, v.clone());
            }
            vc
        };

        // Use default private_key (safe as it's only used for re-execution)
        chain_config.private_key = ZeroingPrivateKey::default();

        // Reconstruct VersionedConstants from protocol version
        let protocol_version: StarknetVersion =
            serializable.protocol_version.parse().context("Failed to parse protocol version")?;
        let exec_constants = backend_chain_config.versioned_constants.0.get(&protocol_version).cloned().context(
            format!("Failed to find VersionedConstants for protocol version {}", serializable.protocol_version),
        )?;

        Ok(Self { chain_config, exec_constants, no_charge_fee: serializable.no_charge_fee })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Load mainnet config for testing (same as chain_config.rs tests)
    fn load_mainnet_config() -> ChainConfig {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("Failed to get CARGO_MANIFEST_DIR");
        let config_path = std::path::Path::new(&manifest_dir).join("../../../../configs/presets/mainnet.yaml");
        ChainConfig::from_yaml(&config_path).expect("Failed to load mainnet config")
    }

    /// Verifies round-trip serialization preserves all config values.
    #[test]
    fn test_round_trip_serialization_preserves_config() {
        let config = Arc::new(load_mainnet_config());
        let exec_constants = config
            .versioned_constants
            .0
            .get(&config.latest_protocol_version)
            .cloned()
            .expect("Mainnet config should have versioned constants");
        let no_charge_fee = false;

        // Create RuntimeExecutionConfig from mainnet config
        let runtime_config =
            RuntimeExecutionConfig::from_arc_chain_config(&config, exec_constants.clone(), no_charge_fee)
                .expect("Failed to create RuntimeExecutionConfig");

        // Serialize to database format
        let serializable = runtime_config.to_serializable().expect("Failed to serialize RuntimeExecutionConfig");

        // Verify version is stored correctly
        assert_eq!(serializable.config_version, config.config_version);

        // Deserialize back
        let deserialized = RuntimeExecutionConfig::from_serializable(serializable, &config)
            .expect("Failed to deserialize RuntimeExecutionConfig");

        // Verify critical fields are preserved
        assert_eq!(deserialized.chain_config.config_version, runtime_config.chain_config.config_version);
        assert_eq!(deserialized.chain_config.chain_name, runtime_config.chain_config.chain_name);
        assert_eq!(deserialized.chain_config.chain_id, runtime_config.chain_config.chain_id);
        assert_eq!(deserialized.chain_config.block_time, runtime_config.chain_config.block_time);
        assert_eq!(deserialized.no_charge_fee, runtime_config.no_charge_fee);
    }
}
