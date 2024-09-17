use std::{str::FromStr, time::Duration};

use mp_block::H160;
use mp_chain_config::{ChainConfig, StarknetVersion};
use starknet_api::{
    contract_address,
    core::{ChainId, ContractAddress, PatriciaKey},
    felt, patricia_key,
};

/// Parameters used to override chain config.
#[derive(Clone, Debug, clap::Parser)]
pub struct ChainConfigOverrideParams {
    //Overrideable args
    #[arg(long, requires = "chain_config_override", value_name = "OVERRIDED CHAIN NAME")]
    pub chain_name: Option<String>,
    #[arg(long, requires = "chain_config_override", value_name = "OVERRIDED CHAIN ID")]
    pub chain_id: Option<String>,
    #[arg(long, requires = "chain_config_override", value_name = "OVERRIDED NATIVE FEE TOKEN")]
    pub native_fee_token_address: Option<String>,
    #[arg(long, requires = "chain_config_override", value_name = "OVERRIDED PARENT FEE TOKEN")]
    pub parent_fee_token_address: Option<String>,
    #[arg(long, requires = "chain_config_override", value_name = "OVERRIDED LATEST PROTOCOL VERSION")]
    pub latest_protocol_version: Option<String>,
    #[arg(long, requires = "chain_config_override", value_name = "OVERRIDED BLOCK TIME")]
    pub block_time: Option<u64>,
    #[arg(long, requires = "chain_config_override", value_name = "OVERRIDED PENDING BLOCK UPDATE")]
    pub pending_block_update_time: Option<u64>,
    #[arg(long, requires = "chain_config_override", value_name = "OVERRIDED SEQUENCER ADDRESS")]
    pub sequencer_address: Option<String>,
    #[arg(long, requires = "chain_config_override", value_name = "OVERRIDED MAX NONCE VALIDATION FOR SKIP")]
    pub max_nonce_for_validation_skip: Option<u64>,
    #[arg(long, requires = "chain_config_override", value_name = "OVERRIDED ETH CORE CONTRACT")]
    pub eth_core_contract_address: Option<String>,
}

impl ChainConfigOverrideParams {
    pub fn override_cfg(&self, mut chain_config: ChainConfig) -> ChainConfig {
        let params = self.clone();
        (
            chain_config.chain_name,
            chain_config.chain_id,
            chain_config.native_fee_token_address,
            chain_config.parent_fee_token_address,
            chain_config.latest_protocol_version,
            chain_config.block_time,
            chain_config.pending_block_update_time,
            chain_config.sequencer_address,
            chain_config.max_nonce_for_validation_skip,
            chain_config.eth_core_contract_address,
        ) = (
            params.chain_name.map_or(chain_config.chain_name, |v| v),
            params.chain_id.map_or(chain_config.chain_id, ChainId::from),
            params
                .native_fee_token_address
                .map_or(chain_config.native_fee_token_address, |v| contract_address!(v.as_str())),
            params
                .parent_fee_token_address
                .map_or(chain_config.parent_fee_token_address, |v| contract_address!(v.as_str())),
            params.latest_protocol_version.map_or(chain_config.latest_protocol_version, |v| {
                StarknetVersion::from_str(v.as_str()).expect("failed to retrieve version")
            }),
            params.block_time.map_or(chain_config.block_time, Duration::from_secs),
            self.pending_block_update_time.map_or(chain_config.pending_block_update_time, Duration::from_secs),
            params.sequencer_address.map_or(chain_config.sequencer_address, |v| contract_address!(v.as_str())),
            params.max_nonce_for_validation_skip.map_or(chain_config.max_nonce_for_validation_skip, |v| v),
            params.eth_core_contract_address.map_or(chain_config.eth_core_contract_address, |v| {
                H160::from_str(v.as_str()).expect("failed to parse core contract")
            }),
        );
        chain_config
    }
}
