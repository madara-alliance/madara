use anyhow::Context;
use mp_block::H160;
use mp_chain_config::{ChainConfig, StarknetVersion};
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use starknet_api::core::{ChainId, ContractAddress};
use std::time::Duration;

/// Override chain config parameters.
/// Format: "--chain-config-override key1=value1 key2=value2"
#[derive(clap::Parser, Clone, Debug)]
pub struct ChainConfigOverrideParams {
    #[clap(long = "chain-config-override", value_parser = parse_key_value, number_of_values = 1)]
    pub overrides: Vec<(String, String)>,
}

impl ChainConfigOverrideParams {
    pub fn override_chain_config(&self, chain_config: ChainConfig) -> anyhow::Result<ChainConfig> {
        let overridable = OverridableChainConfig::from(&chain_config);
        let mut config_value =
            serde_yaml::to_value(overridable).context("Converting OverridableChainConfig to Value")?;

        if let Value::Mapping(ref mut map) = config_value {
            for (key, value) in &self.overrides {
                let value = serde_yaml::from_str(value).with_context(|| format!("Parsing value for key '{}'", key))?;
                map.insert(Value::String(key.clone()), value);
            }
        }

        log::info!("{:?}", config_value);

        let updated_overridable: OverridableChainConfig =
            serde_yaml::from_value(config_value).context("Converting Value to OverridableChainConfig")?;

        Ok(ChainConfig {
            versioned_constants: chain_config.versioned_constants,
            bouncer_config: chain_config.bouncer_config,
            ..updated_overridable.into()
        })
    }
}

fn parse_key_value(s: &str) -> anyhow::Result<(String, String)> {
    let mut parts = s.splitn(2, '=');
    let key = parts.next().ok_or_else(|| anyhow::anyhow!("Invalid key-value pair"))?;
    let value = parts.next().ok_or_else(|| anyhow::anyhow!("Invalid key-value pair"))?;
    Ok((key.to_string(), value.to_string()))
}

/// Part of the Chain Config that we can override.
// We need this proxy structure to implement Serialize -
// which is not possible on the original ChainConfig because the bouncer config and
// the versioned constants don't implement it.
// Since we don't want to override those values anyway, we can create this wrapper.
#[derive(Debug, Serialize, Deserialize)]
pub struct OverridableChainConfig {
    pub chain_name: String,
    pub chain_id: ChainId,
    pub native_fee_token_address: ContractAddress,
    pub parent_fee_token_address: ContractAddress,
    pub latest_protocol_version: StarknetVersion,
    pub block_time: Duration,
    pub pending_block_update_time: Duration,
    pub execution_batch_size: usize,
    pub sequencer_address: ContractAddress,
    pub max_nonce_for_validation_skip: u64,
    pub eth_core_contract_address: H160,
}

impl From<&ChainConfig> for OverridableChainConfig {
    fn from(config: &ChainConfig) -> Self {
        OverridableChainConfig {
            chain_name: config.chain_name.clone(),
            chain_id: config.chain_id.clone(),
            native_fee_token_address: config.native_fee_token_address,
            parent_fee_token_address: config.parent_fee_token_address,
            latest_protocol_version: config.latest_protocol_version,
            block_time: config.block_time,
            pending_block_update_time: config.pending_block_update_time,
            execution_batch_size: config.execution_batch_size,
            sequencer_address: config.sequencer_address,
            max_nonce_for_validation_skip: config.max_nonce_for_validation_skip,
            eth_core_contract_address: config.eth_core_contract_address,
        }
    }
}

impl From<OverridableChainConfig> for ChainConfig {
    fn from(overridable: OverridableChainConfig) -> Self {
        ChainConfig {
            chain_name: overridable.chain_name,
            chain_id: overridable.chain_id,
            native_fee_token_address: overridable.native_fee_token_address,
            parent_fee_token_address: overridable.parent_fee_token_address,
            latest_protocol_version: overridable.latest_protocol_version,
            block_time: overridable.block_time,
            pending_block_update_time: overridable.pending_block_update_time,
            execution_batch_size: overridable.execution_batch_size,
            sequencer_address: overridable.sequencer_address,
            max_nonce_for_validation_skip: overridable.max_nonce_for_validation_skip,
            eth_core_contract_address: overridable.eth_core_contract_address,
            versioned_constants: Default::default(),
            bouncer_config: Default::default(),
        }
    }
}
