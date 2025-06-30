use crate::setup::base_layer::BaseLayerSetupTrait;
use serde::Deserialize;
use std::collections::HashMap;

use crate::setup::base_layer::ethereum::EthereumSetup;
use crate::setup::base_layer::starknet::StarknetSetup;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub base_layer: Option<BaseLayerConfig>,
    pub madara: Option<MadaraConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "layer")]
pub enum BaseLayerConfig {
    Ethereum { rpc_url: String, implementation_address: HashMap<String, String> },
    Starknet { rpc_url: String },
}

#[derive(Debug, Deserialize)]
pub struct MadaraConfig {
    pub rpc_url: String,
}

impl Config {
    pub fn validate(&self) -> color_eyre::Result<()> {
        if self.base_layer.is_none() && self.madara.is_none() {
            Err(color_eyre::eyre::eyre!("At least one of 'base_layer_config' or 'madara_config' must be provided"))
        } else {
            Ok(())
        }
    }

    pub fn get_base_layer_setup(&self, private_key: String) -> anyhow::Result<Box<dyn BaseLayerSetupTrait>> {
        match &self.base_layer {
            Some(BaseLayerConfig::Ethereum { rpc_url, implementation_address }) => {
                Ok(Box::new(EthereumSetup::new(rpc_url.clone(), implementation_address.clone())))
            }
            Some(BaseLayerConfig::Starknet { rpc_url }) => {
                Ok(Box::new(StarknetSetup::new(rpc_url.clone(), private_key)))
            }
            None => Err(anyhow::anyhow!("Base layer config is not provided")),
        }
    }
}
