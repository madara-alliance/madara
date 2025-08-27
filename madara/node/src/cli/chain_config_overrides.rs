use std::time::Duration;

use anyhow::{bail, Context};
use blockifier::bouncer::BouncerConfig;
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use starknet_api::core::{ChainId, ContractAddress};

use mp_chain_config::{
    deserialize_starknet_version, serialize_starknet_version, BlockProductionConfig, ChainConfig,
    L1DataAvailabilityMode, MempoolMode, StarknetVersion,
};
use mp_utils::parsers::parse_key_value_yaml;
use mp_utils::serde::{
    deserialize_duration, deserialize_optional_duration, serialize_duration, serialize_optional_duration,
};
use url::Url;

/// Override chain config parameters.
/// Format: "--chain-config-override chain_id=SN_MADARA,chain_name=MADARA,block_time=1500ms,bouncer_config.block_max_capacity.n_steps=100000000"
#[derive(Parser, Clone, Debug, Deserialize, Serialize)]
pub struct ChainConfigOverrideParams {
    /// Overrides parameters from the chain config.
    ///
    /// Use the following syntax:
    /// --chain-config-override=block_time=30s,chain_id=MADARA_TEST...
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
    ///   * mempool_max_transactions: max number of transactions allowed in the mempool
    ///     in sequencer mode.
    ///
    ///   * mempool_max_declare_transactions: max number of declare transactions allowed
    ///     in sequencer mode.
    ///
    ///   * mempool_ttl: max age of transactions in the mempool.
    ///     Transactions which are too old will be removed.
    #[clap(env = "MADARA_CHAIN_CONFIG_OVERRIDE", long = "chain-config-override", value_parser = parse_key_value_yaml, use_value_delimiter = true, value_delimiter = ',')]
    pub overrides: Vec<(String, Value)>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChainConfigOverridesInner {
    pub chain_name: String,
    pub chain_id: ChainId,
    pub l1_da_mode: L1DataAvailabilityMode,
    pub feeder_gateway_url: Url,
    pub gateway_url: Url,
    pub native_fee_token_address: ContractAddress,
    pub parent_fee_token_address: ContractAddress,
    #[serde(deserialize_with = "deserialize_starknet_version", serialize_with = "serialize_starknet_version")]
    pub latest_protocol_version: StarknetVersion,
    #[serde(deserialize_with = "deserialize_duration", serialize_with = "serialize_duration")]
    pub block_time: Duration,
    pub bouncer_config: BouncerConfig,
    pub sequencer_address: ContractAddress,
    pub eth_core_contract_address: String,
    pub eth_gps_statement_verifier: String,
    #[serde(default)]
    pub mempool_mode: MempoolMode,
    #[serde(default)]
    pub mempool_min_tip_bump: f64,
    pub mempool_max_transactions: usize,
    pub mempool_max_declare_transactions: Option<usize>,
    #[serde(deserialize_with = "deserialize_optional_duration", serialize_with = "serialize_optional_duration")]
    pub mempool_ttl: Option<Duration>,
    pub l2_gas_target: u128,
    pub min_l2_gas_price: u128,
    pub l2_gas_price_max_change_denominator: u128,
    pub no_empty_blocks: bool,
    pub block_production_concurrency: BlockProductionConfig,
    #[serde(deserialize_with = "deserialize_duration", serialize_with = "serialize_duration")]
    pub l1_messages_replay_max_duration: Duration,
}

impl ChainConfigOverrideParams {
    pub fn override_chain_config(&self, chain_config: ChainConfig) -> anyhow::Result<ChainConfig> {
        let versioned_constants = chain_config.versioned_constants;

        let mut chain_config_overrides = serde_yaml::to_value(ChainConfigOverridesInner {
            chain_name: chain_config.chain_name,
            chain_id: chain_config.chain_id,
            l1_da_mode: chain_config.l1_da_mode,
            native_fee_token_address: chain_config.native_fee_token_address,
            parent_fee_token_address: chain_config.parent_fee_token_address,
            latest_protocol_version: chain_config.latest_protocol_version,
            block_time: chain_config.block_time,
            bouncer_config: chain_config.bouncer_config,
            sequencer_address: chain_config.sequencer_address,
            eth_core_contract_address: chain_config.eth_core_contract_address,
            eth_gps_statement_verifier: chain_config.eth_gps_statement_verifier,
            mempool_mode: chain_config.mempool_mode,
            mempool_min_tip_bump: chain_config.mempool_min_tip_bump,
            mempool_max_transactions: chain_config.mempool_max_transactions,
            mempool_max_declare_transactions: chain_config.mempool_max_declare_transactions,
            mempool_ttl: chain_config.mempool_ttl,
            l2_gas_target: chain_config.l2_gas_target,
            min_l2_gas_price: chain_config.min_l2_gas_price,
            l2_gas_price_max_change_denominator: chain_config.l2_gas_price_max_change_denominator,
            feeder_gateway_url: chain_config.feeder_gateway_url,
            gateway_url: chain_config.gateway_url,
            no_empty_blocks: chain_config.no_empty_blocks,
            block_production_concurrency: chain_config.block_production_concurrency,
            l1_messages_replay_max_duration: chain_config.l1_messages_replay_max_duration,
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
            l1_da_mode: chain_config_overrides.l1_da_mode,
            feeder_gateway_url: chain_config_overrides.feeder_gateway_url,
            gateway_url: chain_config_overrides.gateway_url,
            native_fee_token_address: chain_config_overrides.native_fee_token_address,
            parent_fee_token_address: chain_config_overrides.parent_fee_token_address,
            latest_protocol_version: chain_config_overrides.latest_protocol_version,
            block_time: chain_config_overrides.block_time,
            bouncer_config: chain_config_overrides.bouncer_config,
            sequencer_address: chain_config_overrides.sequencer_address,
            eth_core_contract_address: chain_config_overrides.eth_core_contract_address,
            versioned_constants,
            eth_gps_statement_verifier: chain_config_overrides.eth_gps_statement_verifier,
            private_key: chain_config.private_key,
            mempool_mode: chain_config.mempool_mode,
            mempool_min_tip_bump: chain_config.mempool_min_tip_bump,
            mempool_max_transactions: chain_config.mempool_max_transactions,
            mempool_max_declare_transactions: chain_config.mempool_max_declare_transactions,
            mempool_ttl: chain_config.mempool_ttl,
            l2_gas_target: chain_config_overrides.l2_gas_target,
            min_l2_gas_price: chain_config_overrides.min_l2_gas_price,
            l2_gas_price_max_change_denominator: chain_config_overrides.l2_gas_price_max_change_denominator,
            no_empty_blocks: chain_config_overrides.no_empty_blocks,
            block_production_concurrency: chain_config_overrides.block_production_concurrency,
            l1_messages_replay_max_duration: chain_config_overrides.l1_messages_replay_max_duration,
        })
    }
}
