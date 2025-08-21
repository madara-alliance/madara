use crate::setup::base_layer::BaseLayerSetupTrait;
use alloy::signers::local::PrivateKeySigner;
use alloy::{
    network::TransactionBuilder,
    primitives::{Address, Bytes},
    providers::{Provider, ProviderBuilder},
    rpc::types::TransactionRequest,
};
use alloy_sol_types::{sol, SolValue};
use serde_json::Value;
use std::collections::HashMap;
use std::str::FromStr;

#[derive(Clone, Debug)]
pub struct EthereumSetup {
    rpc_url: String,
    signer: PrivateKeySigner,
    implementation_address: HashMap<String, String>,
}

impl EthereumSetup {
    pub fn new(rpc_url: String, private_key: String, implementation_address: HashMap<String, String>) -> Self {
        let signer: PrivateKeySigner = private_key.parse().expect("Failed to parse private key");
        Self { rpc_url, signer, implementation_address }
    }
}

impl BaseLayerSetupTrait for EthereumSetup {
    fn init(&self) -> anyhow::Result<()> {
        Ok(())
    }
    fn setup(&self) -> anyhow::Result<()> {
        Ok(())
    }
    fn post_madara_setup(&self) -> anyhow::Result<()> {
        Ok(())
    }
}
