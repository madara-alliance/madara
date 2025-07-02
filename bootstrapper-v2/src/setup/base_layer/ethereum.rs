use crate::setup::base_layer::BaseLayerSetupTrait;
use std::collections::HashMap;

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct EthereumSetup {
    rpc_url: String,
    implementation_address: HashMap<String, String>,
}

impl EthereumSetup {
    pub fn new(rpc_url: String, implementation_address: HashMap<String, String>) -> Self {
        Self { rpc_url, implementation_address }
    }
}

impl BaseLayerSetupTrait for EthereumSetup {
    fn init(&self) -> color_eyre::Result<()> {
        Ok(())
    }
    fn setup(&self) -> color_eyre::Result<()> {
        Ok(())
    }
    fn post_madara_setup(&self) -> color_eyre::Result<()> {
        Ok(())
    }
}
