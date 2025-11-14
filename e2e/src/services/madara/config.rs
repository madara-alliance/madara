// Can be extened to support all the args present in configs/args/config.json

use crate::services::constants::*;
use crate::services::helpers::{get_binary_path, get_database_path, get_file_path, NodeRpcError};
use crate::services::server::ServerError;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::process::Command;
use url::Url;

#[derive(Debug, thiserror::Error)]
pub enum MadaraError {
    #[error("Binary not found: {0}")]
    BinaryNotFound(String),
    #[error("Server error: {0}")]
    Server(#[from] ServerError),
    #[error("Missing required configuration: {0}")]
    MissingConfig(String),
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
    #[error("Madara connection failed: {0}")]
    ConnectionFailed(String),
    #[error("File system error: {0}")]
    FileSystem(#[from] std::io::Error),
    #[error("RPC error: {0}")]
    RpcError(#[from] NodeRpcError),
}

#[derive(Debug, Clone, PartialEq)]
pub enum MadaraMode {
    FullNode,
    Sequencer,
}

impl std::fmt::Display for MadaraMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MadaraMode::FullNode => write!(f, "full-node"),
            MadaraMode::Sequencer => write!(f, "sequencer"),
        }
    }
}

// Final immutable configuration
#[derive(Debug, Clone)]
pub struct MadaraConfig {
    binary_path: PathBuf,
    name: String,
    database_path: PathBuf,
    rpc_port: u16,
    rpc_admin_port: u16,
    rpc_cors: String,
    rpc_external: bool,
    rpc_admin: bool,
    mode: MadaraMode,
    chain_config_path: Option<PathBuf>,
    block_time: Option<String>,
    feeder_gateway_enable: bool,
    gateway_enable: bool,
    gateway_external: bool,
    gateway_port: u16,
    charge_fee: bool,
    l1_endpoint: Option<Url>,
    l1_gas_price: u64,
    blob_gas_price: u64,
    strk_per_eth: u64,
    environment_vars: HashMap<String, String>,
    additional_args: Vec<String>,
    logs: (bool, bool),
}

impl Default for MadaraConfig {
    fn default() -> Self {
        Self {
            binary_path: get_binary_path(MADARA_BINARY),
            name: MADARA_NAME.to_string(),
            mode: MadaraMode::Sequencer,
            chain_config_path: Some(get_file_path(MADARA_CONFIG)),
            database_path: get_database_path(DATA_DIR, MADARA_DATABASE_DIR),
            rpc_port: MADARA_RPC_PORT,
            rpc_admin_port: MADARA_RPC_ADMIN_PORT,
            rpc_cors: "*".to_string(),
            rpc_external: true,
            rpc_admin: true,
            feeder_gateway_enable: true,
            gateway_enable: true,
            gateway_external: true,
            gateway_port: MADARA_GATEWAY_PORT,
            charge_fee: false,
            block_time: None,
            l1_endpoint: None,
            l1_gas_price: 1,
            blob_gas_price: 1,
            strk_per_eth: 1,
            environment_vars: HashMap::new(),
            additional_args: Vec::new(),
            logs: (true, true),
        }
    }
}

impl MadaraConfig {
    /// Create a new configuration with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a builder for MadaraConfig using current configuration values
    pub fn builder(&self) -> MadaraConfigBuilder {
        MadaraConfigBuilder { config: self.clone() }
    }

    /// Get the binary path
    pub fn binary_path(&self) -> &PathBuf {
        &self.binary_path
    }

    /// Get the name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the block time
    pub fn block_time(&self) -> Option<String> {
        self.block_time.clone()
    }

    /// Get the database path
    pub fn database_path(&self) -> &PathBuf {
        &self.database_path
    }

    /// Get the RPC port
    pub fn rpc_port(&self) -> u16 {
        self.rpc_port
    }

    /// Get the RPC admin port
    pub fn rpc_admin_port(&self) -> u16 {
        self.rpc_admin_port
    }

    /// Get the RPC CORS
    pub fn rpc_cors(&self) -> &str {
        &self.rpc_cors
    }

    /// Get RPC external setting
    pub fn rpc_external(&self) -> bool {
        self.rpc_external
    }

    /// Get RPC admin setting
    pub fn rpc_admin(&self) -> bool {
        self.rpc_admin
    }

    /// Get the mode
    pub fn mode(&self) -> &MadaraMode {
        &self.mode
    }

    /// Get the chain config path
    pub fn chain_config_path(&self) -> Option<&PathBuf> {
        self.chain_config_path.as_ref()
    }

    /// Get feeder gateway enable setting
    pub fn feeder_gateway_enable(&self) -> bool {
        self.feeder_gateway_enable
    }

    /// Get gateway enable setting
    pub fn gateway_enable(&self) -> bool {
        self.gateway_enable
    }

    /// Get gateway external setting
    pub fn gateway_external(&self) -> bool {
        self.gateway_external
    }

    /// Get the gateway port
    pub fn gateway_port(&self) -> u16 {
        self.gateway_port
    }

    /// Get charge fee setting
    pub fn charge_fee(&self) -> bool {
        self.charge_fee
    }

    /// Get the L1 endpoint
    pub fn l1_endpoint(&self) -> Option<&Url> {
        self.l1_endpoint.as_ref()
    }

    /// Get STRK gas price
    pub fn l1_gas_price(&self) -> u64 {
        self.l1_gas_price
    }

    /// Get blob gas price
    pub fn blob_gas_price(&self) -> u64 {
        self.blob_gas_price
    }

    /// Get environment variables
    pub fn environment_vars(&self) -> &HashMap<String, String> {
        &self.environment_vars
    }

    /// Get additional arguments
    pub fn additional_args(&self) -> &[String] {
        &self.additional_args
    }

    /// Get logs
    pub fn logs(&self) -> (bool, bool) {
        self.logs
    }

    /// Get the rpc endpoint
    pub fn rpc_endpoint(&self) -> Url {
        Url::parse(&format!("http://{}:{}", DEFAULT_SERVICE_HOST, self.rpc_port())).unwrap()
    }

    /// Get the rpc admin endpoint
    pub fn rpc_admin_endpoint(&self) -> Url {
        Url::parse(&format!("http://{}:{}", DEFAULT_SERVICE_HOST, self.rpc_admin_port())).unwrap()
    }

    /// Get the gateway endpoint
    pub fn gateway_endpoint(&self) -> Url {
        Url::parse(&format!("http://{}:{}/{}", DEFAULT_SERVICE_HOST, self.gateway_port(), "gateway")).unwrap()
    }

    /// Get the feeder gateway endpoint
    pub fn feeder_gateway_endpoint(&self) -> Url {
        Url::parse(&format!("http://{}:{}/{}", DEFAULT_SERVICE_HOST, self.gateway_port(), "feeder_gateway")).unwrap()
    }

    /// Convert the configuration to a command
    pub fn to_command(&self) -> Command {
        let binary_path = self.binary_path.to_owned();

        let mut cmd = Command::new(binary_path);

        // Core arguments
        cmd.arg("--name").arg(&self.name);
        cmd.arg("--base-path").arg(&self.database_path);
        cmd.arg("--rpc-port").arg(self.rpc_port.to_string());
        cmd.arg("--rpc-admin-port").arg(self.rpc_admin_port.to_string());
        cmd.arg("--rpc-cors").arg(&self.rpc_cors);
        cmd.arg("--gateway-port").arg(self.gateway_port.to_string());

        // Gas prices
        cmd.arg("--l1-gas-price").arg(self.l1_gas_price.to_string());
        cmd.arg("--blob-gas-price").arg(self.blob_gas_price.to_string());
        cmd.arg("--strk-per-eth").arg(self.strk_per_eth.to_string());

        // Flush every n blocks
        cmd.arg("--flush-every-n-blocks").arg("1");

        // Charge fee flag (inverted logic)
        if !self.charge_fee {
            cmd.arg("--no-charge-fee");
        }

        // Boolean flags
        if self.rpc_external {
            cmd.arg("--rpc-external");
        }
        if self.rpc_admin {
            cmd.arg("--rpc-admin");
        }
        if self.feeder_gateway_enable {
            cmd.arg("--feeder-gateway-enable");
        }
        if self.gateway_enable {
            cmd.arg("--gateway-enable");
        }
        if self.gateway_external {
            cmd.arg("--gateway-external");
        }

        // Mode-specific flags
        match self.mode {
            MadaraMode::Sequencer => {
                cmd.arg("--sequencer");
            }
            MadaraMode::FullNode => {
                cmd.arg("--full");
            }
        }

        if let Some(ref l1_endpoint) = self.l1_endpoint {
            cmd.arg("--l1-endpoint").arg(l1_endpoint.to_string());
        } else {
            cmd.arg("--no-l1-sync");
        }

        // Optional arguments
        if let Some(ref chain_config) = self.chain_config_path {
            cmd.arg("--chain-config-path").arg(chain_config);
        }

        // --chain-config-override block_time=3s
        if let Some(ref block_time) = self.block_time {
            cmd.arg("--chain-config-override").arg(format!("block_time={}", block_time));
        }

        // Additional arguments
        for arg in &self.additional_args {
            cmd.arg(arg);
        }

        // Environment variables
        for (key, value) in &self.environment_vars {
            cmd.env(key, value);
        }

        cmd
    }
}

// Builder type that allows configuration
#[derive(Debug, Clone)]
pub struct MadaraConfigBuilder {
    config: MadaraConfig,
}

impl MadaraConfigBuilder {
    /// Create a new configuration builder with default values
    pub fn new() -> Self {
        Self { config: MadaraConfig::default() }
    }

    /// Build the final immutable configuration
    pub fn build(self) -> MadaraConfig {
        self.config
    }

    /// Set the binary path
    pub fn binary_path(mut self, path: &str) -> Self {
        self.config.binary_path = get_binary_path(path);
        self
    }

    pub fn name(mut self, name: &str) -> Self {
        self.config.name = name.to_string();
        self
    }

    pub fn block_time(mut self, block_time: Option<String>) -> Self {
        self.config.block_time = block_time;
        self
    }

    pub fn database_path<S: AsRef<std::path::Path>>(mut self, path: S) -> Self {
        self.config.database_path = path.as_ref().to_path_buf();
        self
    }

    pub fn rpc_port(mut self, port: u16) -> Self {
        self.config.rpc_port = port;
        self
    }

    pub fn rpc_admin_port(mut self, port: u16) -> Self {
        self.config.rpc_admin_port = port;
        self
    }

    pub fn rpc_cors(mut self, cors: &str) -> Self {
        self.config.rpc_cors = cors.to_string();
        self
    }

    pub fn rpc_external(mut self, external: bool) -> Self {
        self.config.rpc_external = external;
        self
    }

    pub fn rpc_admin(mut self, admin: bool) -> Self {
        self.config.rpc_admin = admin;
        self
    }

    pub fn mode(mut self, mode: MadaraMode) -> Self {
        self.config.mode = mode;
        self
    }

    pub fn chain_config_path<P: Into<PathBuf>>(mut self, path: Option<P>) -> Self {
        self.config.chain_config_path = path.map(|p| p.into());
        self
    }

    pub fn feeder_gateway_enable(mut self, enable: bool) -> Self {
        self.config.feeder_gateway_enable = enable;
        self
    }

    pub fn gateway_enable(mut self, enable: bool) -> Self {
        self.config.gateway_enable = enable;
        self
    }

    pub fn gateway_external(mut self, external: bool) -> Self {
        self.config.gateway_external = external;
        self
    }

    pub fn gateway_port(mut self, port: u16) -> Self {
        self.config.gateway_port = port;
        self
    }

    pub fn charge_fee(mut self, charge_fee: bool) -> Self {
        self.config.charge_fee = charge_fee;
        self
    }

    pub fn l1_endpoint(mut self, endpoint: Option<Url>) -> Self {
        self.config.l1_endpoint = endpoint;
        self
    }

    pub fn l1_gas_price(mut self, price: u64) -> Self {
        self.config.l1_gas_price = price;
        self
    }

    pub fn blob_gas_price(mut self, price: u64) -> Self {
        self.config.blob_gas_price = price;
        self
    }

    pub fn strk_per_eth(mut self, price: u64) -> Self {
        self.config.strk_per_eth = price;
        self
    }

    /// Set the logs
    pub fn logs(mut self, logs: (bool, bool)) -> Self {
        self.config.logs = logs;
        self
    }

    pub fn env_var(mut self, key: &str, value: &str) -> Self {
        self.config.environment_vars.insert(key.to_string(), value.to_string());
        self
    }

    pub fn arg(mut self, arg: &str) -> Self {
        self.config.additional_args.push(arg.to_string());
        self
    }
}

impl Default for MadaraConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}
