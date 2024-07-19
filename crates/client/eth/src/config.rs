use alloy::primitives::Address;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::path::PathBuf;

use crate::error::Error;

/// Default Anvil local endpoint
pub const DEFAULT_RPC_ENDPOINT: &str = "http://127.0.0.1:8545";
/// Default Anvil chain ID
pub const DEFAULT_CHAIN_ID: u64 = 31337;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EthereumClientConfig {
    #[serde(default)]
    pub provider: EthereumProviderConfig,
    #[serde(default)]
    pub contracts: StarknetContracts,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EthereumProviderConfig {
    Http(HttpProviderConfig),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StarknetContracts {
    pub core_contract: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpProviderConfig {
    #[serde(default = "default_rpc_endpoint")]
    pub rpc_endpoint: String,
}

fn default_rpc_endpoint() -> String {
    DEFAULT_RPC_ENDPOINT.into()
}

fn default_chain_id() -> u64 {
    DEFAULT_CHAIN_ID
}

impl Default for HttpProviderConfig {
    fn default() -> Self {
        Self { rpc_endpoint: default_rpc_endpoint() }
    }
}

impl Default for EthereumProviderConfig {
    fn default() -> Self {
        Self::Http(HttpProviderConfig::default())
    }
}

impl EthereumProviderConfig {
    pub fn rpc_endpoint(&self) -> &String {
        match self {
            Self::Http(config) => &config.rpc_endpoint,
        }
    }
}

impl StarknetContracts {
    pub fn core_contract(&self) -> Result<Address, Error> {
        Address::parse_checksummed(&self.core_contract, None).map_err(|e| Error::AddressParseError(e))
    }
}

impl EthereumClientConfig {
    pub fn from_json_file(path: &PathBuf) -> Result<Self, Error> {
        let file = File::open(path).map_err(Error::ConfigReadFromFile)?;
        serde_json::from_reader(file).map_err(Error::ConfigDecodeFromJson)
    }
}
