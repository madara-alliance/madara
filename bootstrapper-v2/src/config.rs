use crate::setup::base_layer::ethereum::config_hash::DEFAULT_CONFIG_HASH_VERSION;
use crate::setup::base_layer::ethereum::factory::Factory;
use crate::setup::base_layer::ethereum::implementation_contracts::ImplementationContract;
use crate::setup::base_layer::BaseLayerSetupTrait;
use serde::Deserialize;
use std::collections::HashMap;

use crate::setup::base_layer::ethereum::EthereumSetup;
use crate::setup::base_layer::starknet::StarknetSetup;

/// Configuration for dynamic config hash calculation
#[derive(Debug, Clone, Deserialize)]
pub struct ConfigHashConfig {
    /// Config hash version (optional, defaults to StarknetOsConfig3)
    #[serde(default = "default_config_hash_version")]
    pub version: String,
    /// Madara chain ID (e.g., "MADARA_DEVNET" or hex "0x4d41444152415f4445564e4554")
    pub madara_chain_id: String,
    /// STRK fee token address on L2
    pub strk_fee_token_address: String,
    /// Optional DA public keys for computing public_keys_hash
    #[serde(default)]
    pub da_public_keys: Vec<String>,
}

fn default_config_hash_version() -> String {
    DEFAULT_CONFIG_HASH_VERSION.to_string()
}

/// Core contract initialization data without configHash (computed at runtime)
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreContractInitDataPartial {
    pub program_hash: alloy::primitives::U256,
    pub aggregator_program_hash: alloy::primitives::U256,
    pub verifier: alloy::primitives::Address,
    pub state: Factory::State,
}

impl CoreContractInitDataPartial {
    /// Builds the full CoreContractInitData with the computed config hash
    pub fn with_config_hash(&self, config_hash: alloy::primitives::U256) -> Factory::CoreContractInitData {
        Factory::CoreContractInitData {
            programHash: self.program_hash,
            aggregatorProgramHash: self.aggregator_program_hash,
            verifier: self.verifier,
            configHash: config_hash,
            state: self.state.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct BaseConfigOuter {
    pub base_layer: BaseLayerConfig,
}

#[derive(Debug, Deserialize)]
pub struct MadaraConfigOuter {
    pub madara: MadaraConfig,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "layer")]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BaseLayerConfig {
    Ethereum {
        rpc_url: String,
        // Addresses of the implementation contracts behind proxies
        // Idea is that these can be reused by just deploying the proxy contract
        // and pointing to the same implementation contract.
        // This would save gas and cost, as the heavy implementation contracts are only deployed once.
        // This a map of the implementation contract name to the implementation contract address.
        implementation_addresses: HashMap<ImplementationContract, String>,
        /// Core contract init data without configHash (computed at runtime)
        core_contract_init_data: Box<CoreContractInitDataPartial>,
        /// Configuration for computing the config hash dynamically
        config_hash_config: ConfigHashConfig,
    },
    Starknet {
        rpc_url: String,
    },
}

#[derive(Debug, Deserialize)]
pub struct MadaraConfig {
    pub rpc_url: String,
}

impl BaseConfigOuter {
    pub fn get_base_layer_setup(
        &self,
        private_key: String,
        addresses_output_path: &str,
    ) -> Box<dyn BaseLayerSetupTrait> {
        match &self.base_layer {
            BaseLayerConfig::Ethereum {
                rpc_url,
                implementation_addresses,
                core_contract_init_data,
                config_hash_config,
            } => Box::new(EthereumSetup::new(
                rpc_url.clone(),
                private_key,
                implementation_addresses.clone(),
                *core_contract_init_data.clone(),
                config_hash_config.clone(),
                addresses_output_path,
            )),
            BaseLayerConfig::Starknet { rpc_url } => Box::new(StarknetSetup::new(rpc_url.clone(), private_key)),
        }
    }
}
