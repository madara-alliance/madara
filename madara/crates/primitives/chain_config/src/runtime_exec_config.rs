//! Runtime execution configuration for block production.
//!
//! Provides types and serialization logic to persist execution configuration when starting a new block.
//! This ensures consistent re-execution on node restarts, even if the configuration changes.
//!
//! ## What Gets Saved
//!
//! - `chain_config` fields (via YAML serialization)
//! - `protocol_version` (string, e.g., "0.13.2") - used to look up versioned constants on restart
//! - `no_charge_fee` flag
//! - `config_version` - for compatibility validation
//!
//! ## What Does NOT Get Saved (and why)
//!
//! - **`versioned_constants`**: The `VersionedConstants` type from blockifier does not implement
//!   `Serialize`, so it cannot be persisted directly. Instead, we save the `protocol_version` and
//!   reconstruct the constants from the current node's configuration on restart.
//!
//! - **`private_key`**: Set to `None` in saved configs. Re-execution doesn't require signing;
//!   the current node's private key (from CLI/chain-config) is used for any signing operations.
//!
//! ## Important Assumption
//!
//! **Versioned constants are immutable for a given protocol version.** This means:
//! - For protocol version "0.14.0", the gas costs, limits, and other constants are the same
//!   across all node versions and restarts.
//! - If you use custom `versioned_constants` (via `versioned_constants_path` in chain config),
//!   you must ensure they remain consistent across restarts.
//! - If this assumption is violated (e.g., custom constants change between runs), re-execution
//!   may produce different results than the original execution.
//!
//! ## Round-Trip Serialization
//!
//! `ChainConfig` does not implement `Clone`, so we use YAML round-trip serialization to create
//! copies. This also automatically excludes `#[serde(skip)]` fields (`private_key`, `versioned_constants`).

use crate::{ChainConfig, ChainConfigVersioned, ChainVersionedConstants, StarknetVersion};
use anyhow::{Context, Result};
use blockifier::blockifier_versioned_constants::VersionedConstants;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Runtime execution configuration for consistent block re-execution.
///
/// This config is saved when starting a new block and loaded on restart to ensure
/// that re-execution uses the exact same parameters as the original execution.
#[derive(Debug)]
pub struct RuntimeExecutionConfig {
    /// Chain config used during execution (without private_key and versioned_constants)
    pub chain_config: ChainConfig,
    /// Execution constants resolved from protocol_version
    pub exec_constants: VersionedConstants,
    /// Whether fees were charged during execution
    pub no_charge_fee: bool,
}

/// Serializable form for database storage.
#[derive(Serialize, Deserialize)]
pub struct RuntimeExecutionConfigSerializable {
    /// Config version for validation on restart
    config_version: u32,
    /// YAML-serialized chain config (excludes private_key and versioned_constants)
    chain_config_yaml: String,
    /// Protocol version string (e.g., "0.13.2") - used to look up exec_constants on restart
    protocol_version: String,
    /// Fee charging flag
    no_charge_fee: bool,
}

impl RuntimeExecutionConfig {
    /// Creates a runtime config from the current chain config.
    ///
    /// This is called when starting a new block to save the execution parameters.
    pub fn from_current_config(
        chain_config: &Arc<ChainConfig>,
        exec_constants: VersionedConstants,
        no_charge_fee: bool,
    ) -> Result<Self> {
        // Create a copy of chain_config for saving
        // Note: private_key and versioned_constants will be excluded during serialization
        let config_for_saving = Self::prepare_chain_config_for_saving(chain_config)?;

        Ok(Self { chain_config: config_for_saving, exec_constants, no_charge_fee })
    }

    /// Prepares a chain config for saving by creating a copy.
    ///
    /// Uses YAML round-trip serialization because `ChainConfig` doesn't implement `Clone`.
    /// This approach also automatically:
    /// - Excludes `private_key` (has `#[serde(skip)]`)
    /// - Excludes `versioned_constants` (has `#[serde(skip)]`)
    fn prepare_chain_config_for_saving(config: &ChainConfig) -> Result<ChainConfig> {
        // Serialize to YAML - this excludes #[serde(skip)] fields
        let yaml_str = serde_yaml::to_string(config).context("Failed to serialize ChainConfig to YAML")?;

        // Deserialize back through versioned enum (handles any version migrations)
        let versioned: ChainConfigVersioned =
            serde_yaml::from_str(&yaml_str).context("Failed to deserialize ChainConfig from YAML")?;

        // Convert to canonical ChainConfig
        let mut chain_config = ChainConfig::try_from(versioned).context("Failed to convert to ChainConfig")?;

        // Copy versioned_constants back (needed for in-memory operations, not persisted)
        chain_config.versioned_constants = {
            let mut vc = ChainVersionedConstants::default();
            for (version, constants) in &config.versioned_constants.0 {
                vc.add(*version, constants.clone());
            }
            vc
        };

        // Explicitly set private_key to None for clarity
        chain_config.private_key = None;

        Ok(chain_config)
    }

    /// Serializes this config for database storage.
    pub fn to_serializable(&self) -> Result<RuntimeExecutionConfigSerializable> {
        let chain_config_yaml = serde_yaml::to_string(&self.chain_config).context("Failed to serialize ChainConfig")?;

        Ok(RuntimeExecutionConfigSerializable {
            config_version: self.chain_config.config_version,
            chain_config_yaml,
            protocol_version: self.chain_config.latest_protocol_version.to_string(),
            no_charge_fee: self.no_charge_fee,
        })
    }

    /// Reconstructs a runtime config from saved data.
    ///
    /// This is called on restart to load the saved execution parameters.
    /// It validates the saved config and reconstructs the missing pieces from the current node.
    pub fn from_saved_config(
        saved: RuntimeExecutionConfigSerializable,
        current_backend_config: &ChainConfig,
    ) -> Result<Self> {
        // Step 1: Deserialize saved chain config first (needed for validation)
        let mut chain_config = Self::deserialize_saved_chain_config(&saved.chain_config_yaml)?;

        // Step 2: Validate saved config against current node (includes chain ID check)
        Self::validate_saved_config(&saved, current_backend_config, &chain_config)?;

        // Step 3: Reconstruct versioned_constants from current node
        // Note: versioned_constants cannot be serialized (external type), so we reconstruct
        // from current node's config using the saved protocol_version
        chain_config.versioned_constants = Self::copy_versioned_constants_from_current(current_backend_config);

        // Step 4: Set private_key to None (re-execution doesn't need it)
        chain_config.private_key = None;

        // Step 5: Look up exec_constants using saved protocol_version
        let protocol_version: StarknetVersion =
            saved.protocol_version.parse().context("Failed to parse saved protocol version")?;

        let exec_constants =
            current_backend_config.versioned_constants.0.get(&protocol_version).cloned().with_context(|| {
                format!(
                    "Protocol version '{}' not found in current node's versioned constants. \
                     This protocol version was used when the block was created, but the current \
                     node doesn't have constants for it.",
                    saved.protocol_version
                )
            })?;

        Ok(Self { chain_config, exec_constants, no_charge_fee: saved.no_charge_fee })
    }

    /// Validates that the saved config is compatible with the current node.
    fn validate_saved_config(
        saved: &RuntimeExecutionConfigSerializable,
        current: &ChainConfig,
        saved_chain_config: &ChainConfig,
    ) -> Result<()> {
        // Check config version - must match exactly
        if saved.config_version != current.config_version {
            anyhow::bail!(
                "Config version mismatch: saved config has version {}, but current node expects version {}. \
                 You may need to migrate your database or use a compatible node version.",
                saved.config_version,
                current.config_version
            );
        }

        // Check chain ID - must match exactly
        if saved_chain_config.chain_id != current.chain_id {
            anyhow::bail!(
                "Chain ID mismatch: saved config has chain_id={}, but current node has chain_id={}. \
                 Cannot re-execute blocks from a different chain.",
                saved_chain_config.chain_id,
                current.chain_id
            );
        }

        Ok(())
    }

    /// Deserializes a saved chain config from YAML.
    fn deserialize_saved_chain_config(yaml: &str) -> Result<ChainConfig> {
        let versioned: ChainConfigVersioned =
            serde_yaml::from_str(yaml).context("Failed to deserialize saved ChainConfig from YAML")?;

        ChainConfig::try_from(versioned).context("Failed to convert saved config to ChainConfig")
    }

    /// Copies versioned_constants from the current node's config.
    fn copy_versioned_constants_from_current(current: &ChainConfig) -> ChainVersionedConstants {
        let mut vc = ChainVersionedConstants::default();
        for (version, constants) in &current.versioned_constants.0 {
            vc.add(*version, constants.clone());
        }
        vc
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Load mainnet config for testing
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

        // Create RuntimeExecutionConfig from current config
        let runtime_config =
            RuntimeExecutionConfig::from_current_config(&config, exec_constants.clone(), no_charge_fee)
                .expect("Failed to create RuntimeExecutionConfig");

        // Serialize to database format
        let serializable = runtime_config.to_serializable().expect("Failed to serialize RuntimeExecutionConfig");

        // Verify version is stored correctly
        assert_eq!(serializable.config_version, config.config_version);

        // Deserialize back (simulating restart)
        let deserialized = RuntimeExecutionConfig::from_saved_config(serializable, &config)
            .expect("Failed to deserialize RuntimeExecutionConfig");

        // Verify critical fields are preserved
        assert_eq!(deserialized.chain_config.config_version, runtime_config.chain_config.config_version);
        assert_eq!(deserialized.chain_config.chain_name, runtime_config.chain_config.chain_name);
        assert_eq!(deserialized.chain_config.chain_id, runtime_config.chain_config.chain_id);
        assert_eq!(deserialized.chain_config.block_time, runtime_config.chain_config.block_time);
        assert_eq!(deserialized.no_charge_fee, runtime_config.no_charge_fee);
    }
}
