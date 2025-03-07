use std::time::Duration;

use anyhow::{bail, Context};
use blockifier::bouncer::BouncerConfig;
use clap::Parser;
use mp_utils::crypto::ZeroingPrivateKey;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use starknet_api::core::{ChainId, ContractAddress};

use mp_block::H160;
use mp_chain_config::{
    deserialize_bouncer_config, deserialize_starknet_version, serialize_bouncer_config, serialize_starknet_version,
    ChainConfig, StarknetVersion,
};
use mp_utils::parsers::parse_key_value_yaml;
use mp_utils::serde::{
    deserialize_duration, deserialize_optional_duration, deserialize_private_key, serialize_duration,
    serialize_optional_duration,
};
use url::Url;

/// Override chain config parameters.
/// Format: "--chain-config-override chain_id=SN_MADARA,chain_name=MADARA,block_time=1500ms,bouncer_config.block_max_capacity.n_steps=100000000"
#[derive(Parser, Clone, Debug)]
pub struct ChainConfigOverrideParams {
    /// Overrides parameters from the chain config.
    ///
    /// Use the following syntax:
    /// --chain-config-override=block_time=30s,pending_block_update_time=2s...
    ///
    /// Parameters:
    ///
    ///   * chain_name: the name of the chain.
    ///
    ///   * chain_id: unique identifier for the chain, for example 'SN_MAIN'.
    ///
    ///   * feeder_gateway_url: default fgw for this chain.
    ///
    ///   * gateway url: default gw for this chain.
    ///
    ///   * native_fee_token_address: on-chain address of this chain's native
    ///     token
    ///
    ///   * parent_fee_token_address: on-chain address of the native token of
    ///     this chain's settlement layer.
    ///
    ///   * latest_protocol_version: latest version of the chain, update on new
    ///     method release, consensus change, etc...
    ///
    ///   * block_time: time it takes to close a block.
    ///
    ///   * pending_block_update_time: time interval at which the pending block
    ///     is updated. This is also referred to internally as a 'tick'. The
    ///     block update time should always be inferior to the block time.
    ///
    ///   * execution_batch_size: number of transaction to process in a single
    ///     tick.
    ///
    ///   * bouncer_config: execution limits per block. This has to be
    ///     yaml-encoded following the format in yaml chain config files.
    ///
    ///   * sequencer_address: the address of this chain's sequencer.
    ///
    ///   * eth_core_contract_address: address of the core contract on the
    ///     settlement layer.
    ///
    ///   * eth_gps_statement_verifier: address of the verifier contract on the
    ///     settlement layer.
    ///
    ///   * private_key: private key used by the node in sequencer mode to sign
    ///     the blocks it provides. This is zeroed.
    ///
    ///   * mempool_tx_limit: max number of transactions allowed in the mempool
    ///     in sequencer mode.
    ///
    ///   * mempool_declare_tx_limit: max number of declare transactions allowed
    ///     in sequencer mode.
    ///
    ///   * mempool_tx_max_age: max age of transactions in the mempool.
    ///     Transactions which are too old will be removed.
    #[clap(env = "MADARA_CHAIN_CONFIG_OVERRIDE", long = "chain-config-override", value_parser = parse_key_value_yaml, use_value_delimiter = true, value_delimiter = ',')]
    pub overrides: Vec<(String, Value)>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChainConfigOverridesInner {
    pub chain_name: String,
    pub chain_id: ChainId,
    pub feeder_gateway_url: Url,
    pub gateway_url: Url,
    pub native_fee_token_address: ContractAddress,
    pub parent_fee_token_address: ContractAddress,
    #[serde(deserialize_with = "deserialize_starknet_version", serialize_with = "serialize_starknet_version")]
    pub latest_protocol_version: StarknetVersion,
    #[serde(deserialize_with = "deserialize_duration", serialize_with = "serialize_duration")]
    pub block_time: Duration,
    #[serde(deserialize_with = "deserialize_duration", serialize_with = "serialize_duration")]
    pub pending_block_update_time: Duration,
    pub execution_batch_size: usize,
    #[serde(deserialize_with = "deserialize_bouncer_config", serialize_with = "serialize_bouncer_config")]
    pub bouncer_config: BouncerConfig,
    pub sequencer_address: ContractAddress,
    pub eth_core_contract_address: H160,
    pub eth_gps_statement_verifier: H160,
    #[serde(default)]
    #[serde(skip_serializing)]
    #[serde(deserialize_with = "deserialize_private_key")]
    pub private_key: ZeroingPrivateKey,
    pub mempool_tx_limit: usize,
    pub mempool_declare_tx_limit: usize,
    #[serde(deserialize_with = "deserialize_optional_duration", serialize_with = "serialize_optional_duration")]
    pub mempool_tx_max_age: Option<Duration>,
}

impl ChainConfigOverrideParams {
    pub fn override_chain_config(&self, chain_config: ChainConfig) -> anyhow::Result<ChainConfig> {
        let versioned_constants = chain_config.versioned_constants;

        let mut chain_config_overrides = serde_yaml::to_value(ChainConfigOverridesInner {
            chain_name: chain_config.chain_name,
            chain_id: chain_config.chain_id,
            native_fee_token_address: chain_config.native_fee_token_address,
            parent_fee_token_address: chain_config.parent_fee_token_address,
            latest_protocol_version: chain_config.latest_protocol_version,
            block_time: chain_config.block_time,
            pending_block_update_time: chain_config.pending_block_update_time,
            execution_batch_size: chain_config.execution_batch_size,
            bouncer_config: chain_config.bouncer_config,
            sequencer_address: chain_config.sequencer_address,
            eth_core_contract_address: chain_config.eth_core_contract_address,
            eth_gps_statement_verifier: chain_config.eth_gps_statement_verifier,
            private_key: chain_config.private_key,
            mempool_tx_limit: chain_config.mempool_tx_limit,
            mempool_declare_tx_limit: chain_config.mempool_declare_tx_limit,
            mempool_tx_max_age: chain_config.mempool_tx_max_age,
            feeder_gateway_url: chain_config.feeder_gateway_url,
            gateway_url: chain_config.gateway_url,
        })
        .context("Failed to convert ChainConfig to Value")?;

        for (key, value) in &self.overrides {
            // Split the key by '.' to handle nested fields
            let key_parts = key.split('.').collect::<Vec<_>>();

            // Navigate to the last field in the path
            let mut current_value = &mut chain_config_overrides;
            for part in key_parts.iter().take(key_parts.len() - 1) {
                current_value = match current_value.get_mut(part) {
                    Some(v) => v,
                    None => bail!("Invalid chain config override key path: {}", key),
                };
            }

            // Set the value to the final field in the path
            let last_key =
                key_parts.last().with_context(|| format!("Invalid chain config override key path: {}", key))?;
            match current_value.get_mut(*last_key) {
                Some(field) => {
                    *field = value.clone();
                }
                None => {
                    bail!("Invalid chain config override key path: {}", key);
                }
            }
        }

        let chain_config_overrides: ChainConfigOverridesInner = serde_yaml::from_value(chain_config_overrides)
            .context("Failed to convert Value to ChainConfigOverridesInner")?;

        Ok(ChainConfig {
            chain_name: chain_config_overrides.chain_name,
            chain_id: chain_config_overrides.chain_id,
            feeder_gateway_url: chain_config_overrides.feeder_gateway_url,
            gateway_url: chain_config_overrides.gateway_url,
            native_fee_token_address: chain_config_overrides.native_fee_token_address,
            parent_fee_token_address: chain_config_overrides.parent_fee_token_address,
            latest_protocol_version: chain_config_overrides.latest_protocol_version,
            block_time: chain_config_overrides.block_time,
            pending_block_update_time: chain_config_overrides.pending_block_update_time,
            execution_batch_size: chain_config_overrides.execution_batch_size,
            bouncer_config: chain_config_overrides.bouncer_config,
            sequencer_address: chain_config_overrides.sequencer_address,
            eth_core_contract_address: chain_config_overrides.eth_core_contract_address,
            versioned_constants,
            eth_gps_statement_verifier: chain_config_overrides.eth_gps_statement_verifier,
            private_key: chain_config_overrides.private_key,
            mempool_tx_limit: chain_config_overrides.mempool_tx_limit,
            mempool_declare_tx_limit: chain_config_overrides.mempool_declare_tx_limit,
            mempool_tx_max_age: chain_config_overrides.mempool_tx_max_age,
        })
    }
}
