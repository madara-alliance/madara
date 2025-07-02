use crate::setup::base_layer::BaseLayerSetupTrait;
use serde::Deserialize;
use std::collections::HashMap;

use crate::setup::base_layer::ethereum::EthereumSetup;
use crate::setup::base_layer::starknet::StarknetSetup;

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
    Ethereum { rpc_url: String, implementation_addresses: HashMap<String, String> },
    Starknet { rpc_url: String },
}

#[derive(Debug, Deserialize)]
pub struct MadaraConfig {
    pub rpc_url: String,
}

impl BaseConfigOuter {
    pub fn get_base_layer_setup(&self, private_key: String) -> anyhow::Result<Box<dyn BaseLayerSetupTrait>> {
        match &self.base_layer {
            BaseLayerConfig::Ethereum { rpc_url, implementation_addresses } => {
                Ok(Box::new(EthereumSetup::new(rpc_url.clone(), implementation_addresses.clone())))
            }
            BaseLayerConfig::Starknet { rpc_url } => Ok(Box::new(StarknetSetup::new(rpc_url.clone(), private_key))),
        }
    }
}

impl MadaraConfigOuter {
    pub fn get_madara_setup(&self, private_key: String) -> anyhow::Result<Box<dyn BaseLayerSetupTrait>> {
        Ok(Box::new(MadaraSetup::new(self.madara.rpc_url.clone(), private_key)))
    }
}
