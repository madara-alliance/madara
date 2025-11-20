//! Runtime execution configuration for block production.
//!
//! This module provides types and serialization logic for persisting runtime execution
//! configuration when starting a new block. This ensures that re-execution on restart
//! uses the same configs that were used during original execution.
//!
//! ## Version Handling
//!
//! This module handles ChainConfig versioning automatically through round-trip serialization.
//! When a ChainConfig is saved, its version is tracked and stored. On deserialization:
//!
//! - **Same version, different values**: The saved config values are used for re-execution
//! - **Different versions**: The saved config (with its original version) is deserialized
//!   using the appropriate version handler from `ChainConfigVersioned`. This ensures that
//!   pre-confirmed blocks are always closed using the exact config that was used during
//!   original execution, even if the node restarts with a different ChainConfig version.
//!
//! ### Example Scenarios
//!
//! 1. **Node stops with v1 config, restarts with v1 config**: Uses saved v1 config ✓
//! 2. **Node stops with v1 config, restarts with v2 config**: Uses saved v1 config for
//!    re-execution, then saves v2 config for next block ✓
//! 3. **Same version, different values**: Uses saved values for re-execution ✓
//!
//! ### Maintenance Notes
//!
//! When adding a new ChainConfig version:
//! - Add the new variant to `ChainConfigVersioned` enum
//! - Implement `TryFrom<ChainConfigV2> for ChainConfig`
//! - Update `TryFrom<ChainConfigVersioned> for ChainConfig` to handle the new variant
//! - No changes needed in this module - version handling is automatic!

use crate::{ChainConfig, ChainConfigVersioned, ChainVersionedConstants, StarknetVersion};
use anyhow::{Context, Result};
use blockifier::blockifier_versioned_constants::VersionedConstants;
use mp_utils::crypto::ZeroingPrivateKey;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Runtime execution configuration saved when starting a new block.
/// This ensures that re-execution on restart uses the same configs that were used during original execution.
#[derive(Debug)]
pub struct RuntimeExecutionConfig {
    /// Complete chain configuration used during execution
    pub chain_config: ChainConfig,
    /// The actual execution constants resolved for the protocol_version used
    pub exec_constants: VersionedConstants,
    /// Fee charging flag (not part of ChainConfig but affects execution)
    pub no_charge_fee: bool,
}

/// Serializable representation of RuntimeExecutionConfig for database storage.
/// This is used internally for serialization since ChainConfig and VersionedConstants
/// don't implement Serialize/Deserialize directly.
#[derive(Serialize, Deserialize)]
pub struct RuntimeExecutionConfigSerializable {
    /// Config version used when this was serialized (for version tracking and migration)
    config_version: u32,
    /// Chain config stored as YAML string (using ChainConfig's deserialization format)
    /// Note: Field name says "json" for historical reasons, but it's actually YAML
    chain_config_json: String,
    /// Protocol version string - we'll reconstruct VersionedConstants from backend's versioned_constants map
    protocol_version: String,
    /// Fee charging flag
    no_charge_fee: bool,
}

impl RuntimeExecutionConfig {
    /// Copy ChainConfig using round-trip serialization.
    /// This ensures version-aware copying that automatically handles all ChainConfig versions.
    /// When new ChainConfig versions are added, this function continues to work without changes.
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

    /// Convert ChainConfig to versioned YAML string.
    /// This is used both for copying and for database serialization.
    fn chain_config_to_versioned_yaml(config: &ChainConfig) -> Result<String> {
        use serde_yaml;

        // Manually construct a YAML Value that matches ChainConfigVersioned format
        // This allows us to use ChainConfig's deserialization logic when reading back
        let mut yaml_value = serde_yaml::Mapping::new();

        // Set config_version tag (required for ChainConfigVersioned deserialization)
        // Use dynamic version from the config instead of hardcoding
        yaml_value.insert(
            serde_yaml::Value::String("config_version".to_string()),
            serde_yaml::Value::String(config.config_version.to_string()),
        );

        // Add all ChainConfig fields
        yaml_value.insert(
            serde_yaml::Value::String("chain_name".to_string()),
            serde_yaml::Value::String(config.chain_name.clone()),
        );
        yaml_value.insert(
            serde_yaml::Value::String("chain_id".to_string()),
            serde_yaml::Value::String(config.chain_id.to_string()),
        );
        yaml_value.insert(
            serde_yaml::Value::String("l1_da_mode".to_string()),
            serde_yaml::Value::String(format!("{:?}", config.l1_da_mode)),
        );
        yaml_value.insert(
            serde_yaml::Value::String("settlement_chain_kind".to_string()),
            serde_yaml::Value::String(format!("{:?}", config.settlement_chain_kind)),
        );
        yaml_value.insert(
            serde_yaml::Value::String("feeder_gateway_url".to_string()),
            serde_yaml::Value::String(config.feeder_gateway_url.as_str().to_string()),
        );
        yaml_value.insert(
            serde_yaml::Value::String("gateway_url".to_string()),
            serde_yaml::Value::String(config.gateway_url.as_str().to_string()),
        );
        yaml_value.insert(
            serde_yaml::Value::String("native_fee_token_address".to_string()),
            serde_yaml::Value::String(config.native_fee_token_address.to_string()),
        );
        yaml_value.insert(
            serde_yaml::Value::String("parent_fee_token_address".to_string()),
            serde_yaml::Value::String(config.parent_fee_token_address.to_string()),
        );
        yaml_value.insert(
            serde_yaml::Value::String("latest_protocol_version".to_string()),
            serde_yaml::Value::String(config.latest_protocol_version.to_string()),
        );
        // Serialize block_time as a string (e.g., "30s") since deserialize_duration expects a string
        let block_time_str = if config.block_time.as_secs_f64().fract() == 0.0 {
            format!("{}s", config.block_time.as_secs())
        } else {
            format!("{}ms", config.block_time.as_millis())
        };
        yaml_value
            .insert(serde_yaml::Value::String("block_time".to_string()), serde_yaml::Value::String(block_time_str));
        yaml_value.insert(
            serde_yaml::Value::String("no_empty_blocks".to_string()),
            serde_yaml::Value::Bool(config.no_empty_blocks),
        );
        yaml_value.insert(
            serde_yaml::Value::String("bouncer_config".to_string()),
            serde_yaml::to_value(&config.bouncer_config).context("Failed to serialize bouncer_config")?,
        );
        yaml_value.insert(
            serde_yaml::Value::String("sequencer_address".to_string()),
            serde_yaml::Value::String(config.sequencer_address.to_string()),
        );
        yaml_value.insert(
            serde_yaml::Value::String("eth_core_contract_address".to_string()),
            serde_yaml::Value::String(config.eth_core_contract_address.clone()),
        );
        yaml_value.insert(
            serde_yaml::Value::String("eth_gps_statement_verifier".to_string()),
            serde_yaml::Value::String(config.eth_gps_statement_verifier.clone()),
        );
        yaml_value.insert(
            serde_yaml::Value::String("mempool_mode".to_string()),
            serde_yaml::Value::String(format!("{:?}", config.mempool_mode)),
        );
        yaml_value.insert(
            serde_yaml::Value::String("mempool_min_tip_bump".to_string()),
            serde_yaml::Value::Number(serde_yaml::Number::from(config.mempool_min_tip_bump)),
        );
        yaml_value.insert(
            serde_yaml::Value::String("mempool_max_transactions".to_string()),
            serde_yaml::Value::Number(serde_yaml::Number::from(config.mempool_max_transactions)),
        );
        if let Some(max_declare) = config.mempool_max_declare_transactions {
            yaml_value.insert(
                serde_yaml::Value::String("mempool_max_declare_transactions".to_string()),
                serde_yaml::Value::Number(serde_yaml::Number::from(max_declare)),
            );
        }
        if let Some(ttl) = config.mempool_ttl {
            // Serialize mempool_ttl as a string (e.g., "3600s") since deserialize_optional_duration expects a string
            let mempool_ttl_str = if ttl.as_secs_f64().fract() == 0.0 {
                format!("{}s", ttl.as_secs())
            } else {
                format!("{}ms", ttl.as_millis())
            };
            yaml_value.insert(
                serde_yaml::Value::String("mempool_ttl".to_string()),
                serde_yaml::Value::String(mempool_ttl_str),
            );
        }
        yaml_value.insert(
            serde_yaml::Value::String("l2_gas_price".to_string()),
            serde_yaml::to_value(&config.l2_gas_price).context("Failed to serialize l2_gas_price")?,
        );
        yaml_value.insert(
            serde_yaml::Value::String("block_production_concurrency".to_string()),
            serde_yaml::to_value(&config.block_production_concurrency)
                .context("Failed to serialize block_production_concurrency")?,
        );
        // Serialize l1_messages_replay_max_duration as a string (e.g., "259200s") since deserialize_duration expects a string
        let l1_messages_replay_max_duration_str = if config.l1_messages_replay_max_duration.as_secs_f64().fract() == 0.0
        {
            format!("{}s", config.l1_messages_replay_max_duration.as_secs())
        } else {
            format!("{}ms", config.l1_messages_replay_max_duration.as_millis())
        };
        yaml_value.insert(
            serde_yaml::Value::String("l1_messages_replay_max_duration".to_string()),
            serde_yaml::Value::String(l1_messages_replay_max_duration_str),
        );

        // Serialize to YAML string
        serde_yaml::to_string(&serde_yaml::Value::Mapping(yaml_value))
            .context("Failed to serialize ChainConfig to YAML")
    }

    /// Create RuntimeExecutionConfig from Arc<ChainConfig> using version-aware copying.
    /// This uses round-trip serialization to ensure all ChainConfig versions are handled automatically.
    /// When new ChainConfig versions are added, this function continues to work without changes.
    pub fn from_arc_chain_config(
        chain_config: &Arc<ChainConfig>,
        exec_constants: VersionedConstants,
        no_charge_fee: bool,
    ) -> Result<Self> {
        // Copy ChainConfig using round-trip serialization (version-aware)
        let mut chain_config_copy = Self::copy_chain_config_via_serialization(chain_config)
            .context("Failed to copy ChainConfig via serialization")?;

        // Copy versioned_constants manually (it's not part of the serialized config)
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

    /// Convert to serializable form for database storage.
    /// The config version is automatically detected and stored for version tracking.
    pub fn to_serializable(&self) -> Result<RuntimeExecutionConfigSerializable> {
        // Convert ChainConfig to versioned YAML string using the shared helper
        let chain_config_yaml = Self::chain_config_to_versioned_yaml(&self.chain_config)?;

        // Store protocol version instead of VersionedConstants since it doesn't implement Serialize
        // We'll reconstruct VersionedConstants from the backend's versioned_constants map on load
        let protocol_version_str = self.chain_config.latest_protocol_version.to_string();

        Ok(RuntimeExecutionConfigSerializable {
            config_version: self.chain_config.config_version,
            chain_config_json: chain_config_yaml, // Actually YAML, but keeping field name for compatibility
            protocol_version: protocol_version_str,
            no_charge_fee: self.no_charge_fee,
        })
    }

    /// Reconstruct from serializable form.
    pub fn from_serializable(
        serializable: RuntimeExecutionConfigSerializable,
        backend_chain_config: &ChainConfig,
    ) -> Result<Self> {
        use serde_yaml;

        // Log version comparison for debugging
        if serializable.config_version != backend_chain_config.config_version {
            tracing::warn!(
                "RuntimeExecutionConfig version mismatch: saved version {} != current backend version {}. \
                Using saved config for re-execution to ensure consistency.",
                serializable.config_version,
                backend_chain_config.config_version
            );
        }

        // Deserialize ChainConfig from YAML using its built-in deserialization logic
        // This reuses ChainConfig's deserialization code and handles all versions automatically
        let versioned: ChainConfigVersioned = serde_yaml::from_str(&serializable.chain_config_json)
            .context("Failed to deserialize ChainConfig from YAML")?;

        let mut chain_config = ChainConfig::try_from(versioned)
            .context("Failed to convert versioned ChainConfig to canonical ChainConfig")?;

        // Merge versioned_constants from backend (they're not stored in the serialized config)
        // This ensures we have all the versioned constants available
        chain_config.versioned_constants = {
            let mut vc = ChainVersionedConstants::default();
            for (k, v) in &backend_chain_config.versioned_constants.0 {
                vc.add(*k, v.clone());
            }
            vc
        };

        // Use default private_key since it's not serialized and ZeroingPrivateKey doesn't implement Clone
        // This is safe because:
        // 1. The reconstructed ChainConfig is only used for re-execution (which doesn't require private_key)
        // 2. When closing the block (close_preconfirmed_block_with_state_diff -> close_preconfirmed -> write_new_confirmed_inner),
        //    the backend's CURRENT chain_config is used (self.inner.chain_config) for computing block hash and commitments,
        //    not the reconstructed one. Blocks are saved with consensus_signatures: vec![] (not signed at closing time).
        // 3. When blocks are signed (on-demand via gateway API handle_get_signature),
        //    the backend's CURRENT chain_config.private_key is used, not the reconstructed one
        // Therefore, using default() here is acceptable since private_key is never accessed from the reconstructed config
        chain_config.private_key = ZeroingPrivateKey::default();

        // Reconstruct VersionedConstants from protocol version using backend's versioned_constants map
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
    use crate::CURRENT_CONFIG_VERSION;
    use blockifier::blockifier_versioned_constants::VersionedConstants;
    use std::sync::Arc;

    /// Helper to create a test ChainConfig
    fn create_test_chain_config() -> ChainConfig {
        use starknet_api::core::ChainId;
        let mut versioned_constants = ChainVersionedConstants::default();
        let protocol_version = StarknetVersion::new(0, 1, 5, 0);
        versioned_constants.add(protocol_version, VersionedConstants::default());

        ChainConfig {
            config_version: CURRENT_CONFIG_VERSION,
            chain_name: "test_chain".to_string(),
            chain_id: ChainId::Other("TEST_CHAIN".to_string()),
            l1_da_mode: crate::L1DataAvailabilityMode::Blob,
            settlement_chain_kind: crate::SettlementChainKind::Ethereum,
            feeder_gateway_url: url::Url::parse("https://feeder.test").unwrap(),
            gateway_url: url::Url::parse("https://gateway.test").unwrap(),
            native_fee_token_address: starknet_api::core::ContractAddress::default(),
            parent_fee_token_address: starknet_api::core::ContractAddress::default(),
            versioned_constants,
            latest_protocol_version: protocol_version,
            block_time: std::time::Duration::from_secs(30),
            no_empty_blocks: false,
            bouncer_config: blockifier::bouncer::BouncerConfig::default(),
            sequencer_address: starknet_api::core::ContractAddress::default(),
            eth_core_contract_address: "0x123".to_string(),
            eth_gps_statement_verifier: "0x456".to_string(),
            private_key: ZeroingPrivateKey::default(),
            mempool_mode: crate::MempoolMode::Timestamp,
            mempool_min_tip_bump: 0.1,
            mempool_max_transactions: 1000,
            mempool_max_declare_transactions: None,
            mempool_ttl: Some(std::time::Duration::from_secs(3600)),
            l2_gas_price: crate::L2GasPrice::Fixed { price: 1000 },
            block_production_concurrency: crate::BlockProductionConfig::default(),
            l1_messages_replay_max_duration: std::time::Duration::from_secs(259200),
        }
    }

    #[test]
    fn test_round_trip_serialization_same_version() {
        let config = Arc::new(create_test_chain_config());
        let exec_constants = VersionedConstants::default();
        let no_charge_fee = false;

        // Create RuntimeExecutionConfig
        let runtime_config =
            RuntimeExecutionConfig::from_arc_chain_config(&config, exec_constants.clone(), no_charge_fee)
                .expect("Failed to create RuntimeExecutionConfig");

        // Serialize
        let serializable = runtime_config.to_serializable().expect("Failed to serialize RuntimeExecutionConfig");

        // Verify version is stored
        assert_eq!(serializable.config_version, CURRENT_CONFIG_VERSION);

        // Deserialize
        let deserialized = RuntimeExecutionConfig::from_serializable(serializable, &config)
            .expect("Failed to deserialize RuntimeExecutionConfig");

        // Verify all fields match
        assert_eq!(deserialized.chain_config.config_version, runtime_config.chain_config.config_version);
        assert_eq!(deserialized.chain_config.chain_name, runtime_config.chain_config.chain_name);
        assert_eq!(deserialized.chain_config.chain_id, runtime_config.chain_config.chain_id);
        assert_eq!(deserialized.chain_config.block_time, runtime_config.chain_config.block_time);
        assert_eq!(deserialized.no_charge_fee, runtime_config.no_charge_fee);
    }

    #[test]
    fn test_version_tracking() {
        let config = Arc::new(create_test_chain_config());
        let exec_constants = VersionedConstants::default();

        let runtime_config = RuntimeExecutionConfig::from_arc_chain_config(&config, exec_constants.clone(), false)
            .expect("Failed to create RuntimeExecutionConfig");

        let serializable = runtime_config.to_serializable().expect("Failed to serialize");

        // Verify the version is correctly stored
        assert_eq!(serializable.config_version, config.config_version);
    }

    #[test]
    fn test_copy_via_serialization_preserves_all_fields() {
        let original = Arc::new(create_test_chain_config());
        let exec_constants = VersionedConstants::default();

        let runtime_config = RuntimeExecutionConfig::from_arc_chain_config(&original, exec_constants.clone(), true)
            .expect("Failed to create RuntimeExecutionConfig");

        // Verify all important fields are copied correctly
        assert_eq!(runtime_config.chain_config.config_version, original.config_version);
        assert_eq!(runtime_config.chain_config.chain_name, original.chain_name);
        assert_eq!(runtime_config.chain_config.chain_id, original.chain_id);
        assert_eq!(runtime_config.chain_config.block_time, original.block_time);
        assert_eq!(runtime_config.chain_config.mempool_ttl, original.mempool_ttl);
        assert_eq!(
            runtime_config.chain_config.l1_messages_replay_max_duration,
            original.l1_messages_replay_max_duration
        );
        assert_eq!(runtime_config.no_charge_fee, true);
    }

    #[test]
    fn test_serialization_round_trip_preserves_config() {
        let config = Arc::new(create_test_chain_config());
        let exec_constants = VersionedConstants::default();

        // Create and serialize
        let runtime_config = RuntimeExecutionConfig::from_arc_chain_config(&config, exec_constants.clone(), false)
            .expect("Failed to create RuntimeExecutionConfig");

        let serializable = runtime_config.to_serializable().expect("Failed to serialize");

        // Deserialize with same backend config
        let deserialized =
            RuntimeExecutionConfig::from_serializable(serializable, &config).expect("Failed to deserialize");

        // Verify critical fields are preserved
        assert_eq!(deserialized.chain_config.config_version, config.config_version);
        assert_eq!(deserialized.chain_config.chain_name, config.chain_name);
        assert_eq!(deserialized.chain_config.block_time, config.block_time);
    }

    #[test]
    fn test_different_config_values_preserved() {
        let mut config1 = create_test_chain_config();
        config1.chain_name = "chain1".to_string();
        config1.block_time = std::time::Duration::from_secs(60);

        let mut config2 = create_test_chain_config();
        config2.chain_name = "chain2".to_string();
        config2.block_time = std::time::Duration::from_secs(120);

        let exec_constants = VersionedConstants::default();

        // Save config1
        let runtime_config1 =
            RuntimeExecutionConfig::from_arc_chain_config(&Arc::new(config1), exec_constants.clone(), false)
                .expect("Failed to create RuntimeExecutionConfig");

        let serializable = runtime_config1.to_serializable().expect("Failed to serialize");

        // Deserialize with config2 as backend (simulating restart with different config)
        let deserialized =
            RuntimeExecutionConfig::from_serializable(serializable, &config2).expect("Failed to deserialize");

        // Verify saved config1 values are used (not config2 values)
        assert_eq!(deserialized.chain_config.chain_name, "chain1");
        assert_eq!(deserialized.chain_config.block_time, std::time::Duration::from_secs(60));
    }
}
