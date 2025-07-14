// Can be extened to support all the args present in configs/args/config.json

use crate::servers::server::ServerError;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::process::Command;

const DEFAULT_MADARA_RPC_PORT: u16 = 9944;
const DEFAULT_MADARA_GATEWAY_PORT: u16 = 8080;
const DEFAULT_MADARA_NAME: &str = "madara";
pub const MADARA_DEFAULT_DATABASE_NAME: &str = "madara-db";
pub const DEFAULT_MADARA_BINARY_PATH: &str = "../target/release/madara";

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
    #[error("Invalid response from Madara")]
    InvalidResponse,
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

// Builder type that allows configuration
#[derive(Debug, Clone)]
pub struct MadaraConfigBuilder {
    binary_path: Option<PathBuf>,
    name: String,
    database_path: PathBuf,
    rpc_port: u16,
    rpc_cors: String,
    rpc_external: bool,
    rpc_admin: bool,
    mode: MadaraMode,
    chain_config_path: Option<PathBuf>,
    feeder_gateway_enable: bool,
    gateway_enable: bool,
    gateway_external: bool,
    gateway_port: u16,
    charge_fee: bool,
    l1_endpoint: Option<String>,
    strk_gas_price: u64,
    strk_blob_gas_price: u64,
    gas_price: u64,
    blob_gas_price: u64,
    environment_vars: HashMap<String, String>,
    additional_args: Vec<String>,
}

// Final immutable configuration
#[derive(Debug, Clone)]
pub struct MadaraConfig {
    binary_path: Option<PathBuf>,
    name: String,
    database_path: PathBuf,
    rpc_port: u16,
    rpc_cors: String,
    rpc_external: bool,
    rpc_admin: bool,
    mode: MadaraMode,
    chain_config_path: Option<PathBuf>,
    feeder_gateway_enable: bool,
    gateway_enable: bool,
    gateway_external: bool,
    gateway_port: u16,
    charge_fee: bool,
    l1_endpoint: Option<String>,
    strk_gas_price: u64,
    strk_blob_gas_price: u64,
    gas_price: u64,
    blob_gas_price: u64,
    environment_vars: HashMap<String, String>,
    additional_args: Vec<String>,
}

impl Default for MadaraConfigBuilder {
    fn default() -> Self {
        Self {
            binary_path: Some(PathBuf::from(DEFAULT_MADARA_BINARY_PATH)),
            name: DEFAULT_MADARA_NAME.to_string(),
            database_path: PathBuf::from("./data/madara-db"),
            rpc_port: DEFAULT_MADARA_RPC_PORT,
            rpc_cors: "*".to_string(),
            rpc_external: true,
            rpc_admin: true,
            mode: MadaraMode::Sequencer,
            chain_config_path: Some(PathBuf::from("../configs/presets/devnet.yaml")),
            feeder_gateway_enable: true,
            gateway_enable: true,
            gateway_external: true,
            gateway_port: DEFAULT_MADARA_GATEWAY_PORT,
            charge_fee: false,
            // l1_endpoint: Some("http://127.0.0.1:8545".to_string()),
            l1_endpoint: None,
            strk_gas_price: 0,
            strk_blob_gas_price: 0,
            gas_price: 0,
            blob_gas_price: 0,
            environment_vars: HashMap::new(),
            additional_args: Vec::new(),
        }
    }
}

impl MadaraConfigBuilder {
    /// Create a new configuration builder with default values
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_binary_path<P: Into<PathBuf>>(mut self, path: Option<P>) -> Self {
        self.binary_path = path.map(|p| p.into());
        self
    }

    pub fn with_name(mut self, name: &str) -> Self {
        self.name = name.to_string();
        self
    }

    pub fn with_database_path<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.database_path = path.into();
        self
    }

    pub fn with_rpc_port(mut self, port: u16) -> Self {
        self.rpc_port = port;
        self
    }

    pub fn with_rpc_cors(mut self, cors: &str) -> Self {
        self.rpc_cors = cors.to_string();
        self
    }

    pub fn with_rpc_external(mut self, external: bool) -> Self {
        self.rpc_external = external;
        self
    }

    pub fn with_rpc_admin(mut self, admin: bool) -> Self {
        self.rpc_admin = admin;
        self
    }

    pub fn with_mode(mut self, mode: MadaraMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn with_chain_config_path<P: Into<PathBuf>>(mut self, path: Option<P>) -> Self {
        self.chain_config_path = path.map(|p| p.into());
        self
    }

    pub fn with_feeder_gateway_enable(mut self, enable: bool) -> Self {
        self.feeder_gateway_enable = enable;
        self
    }

    pub fn with_gateway_enable(mut self, enable: bool) -> Self {
        self.gateway_enable = enable;
        self
    }

    pub fn with_gateway_external(mut self, external: bool) -> Self {
        self.gateway_external = external;
        self
    }

    pub fn with_gateway_port(mut self, port: u16) -> Self {
        self.gateway_port = port;
        self
    }

    pub fn with_charge_fee(mut self, charge_fee: bool) -> Self {
        self.charge_fee = charge_fee;
        self
    }

    pub fn with_l1_endpoint(mut self, endpoint: Option<&str>) -> Self {
        self.l1_endpoint = endpoint.map(|v| v.to_string());
        self
    }

    pub fn with_strk_gas_price(mut self, price: u64) -> Self {
        self.strk_gas_price = price;
        self
    }

    pub fn with_strk_blob_gas_price(mut self, price: u64) -> Self {
        self.strk_blob_gas_price = price;
        self
    }

    pub fn with_gas_price(mut self, price: u64) -> Self {
        self.gas_price = price;
        self
    }

    pub fn with_blob_gas_price(mut self, price: u64) -> Self {
        self.blob_gas_price = price;
        self
    }

    pub fn add_env_var(mut self, key: &str, value: &str) -> Self {
        self.environment_vars.insert(key.to_string(), value.to_string());
        self
    }

    pub fn add_arg(mut self, arg: &str) -> Self {
        self.additional_args.push(arg.to_string());
        self
    }

    /// Build the final immutable configuration
    pub fn build(self) -> MadaraConfig {
        MadaraConfig {
            binary_path: self.binary_path,
            name: self.name,
            database_path: self.database_path,
            rpc_port: self.rpc_port,
            rpc_cors: self.rpc_cors,
            rpc_external: self.rpc_external,
            rpc_admin: self.rpc_admin,
            mode: self.mode,
            chain_config_path: self.chain_config_path,
            feeder_gateway_enable: self.feeder_gateway_enable,
            gateway_enable: self.gateway_enable,
            gateway_external: self.gateway_external,
            gateway_port: self.gateway_port,
            charge_fee: self.charge_fee,
            l1_endpoint: self.l1_endpoint,
            strk_gas_price: self.strk_gas_price,
            strk_blob_gas_price: self.strk_blob_gas_price,
            gas_price: self.gas_price,
            blob_gas_price: self.blob_gas_price,
            environment_vars: self.environment_vars,
            additional_args: self.additional_args,
        }
    }
}

impl MadaraConfig {
    /// Get the binary path
    pub fn binary_path(&self) -> Option<&PathBuf> {
        self.binary_path.as_ref()
    }

    /// Get the name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the database path
    pub fn database_path(&self) -> &PathBuf {
        &self.database_path
    }

    /// Get the RPC port
    pub fn rpc_port(&self) -> u16 {
        self.rpc_port
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
    pub fn l1_endpoint(&self) -> Option<&str> {
        self.l1_endpoint.as_deref()
    }

    /// Get STRK gas price
    pub fn strk_gas_price(&self) -> u64 {
        self.strk_gas_price
    }

    /// Get STRK blob gas price
    pub fn strk_blob_gas_price(&self) -> u64 {
        self.strk_blob_gas_price
    }

    /// Get gas price
    pub fn gas_price(&self) -> u64 {
        self.gas_price
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

    /// Convert the configuration to a command
    pub fn to_command(&self) -> Command {
        let binary_path = self.binary_path.as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| DEFAULT_MADARA_BINARY_PATH.to_string());

        let mut cmd = Command::new(binary_path);

        // Core arguments
        cmd.arg("--name").arg(&self.name);
        cmd.arg("--base-path").arg(&self.database_path);
        cmd.arg("--rpc-port").arg(self.rpc_port.to_string());
        cmd.arg("--rpc-cors").arg(&self.rpc_cors);
        cmd.arg("--gateway-port").arg(self.gateway_port.to_string());

        // Gas prices
        cmd.arg("--strk-gas-price").arg(self.strk_gas_price.to_string());
        cmd.arg("--strk-blob-gas-price").arg(self.strk_blob_gas_price.to_string());
        cmd.arg("--gas-price").arg(self.gas_price.to_string());
        cmd.arg("--blob-gas-price").arg(self.blob_gas_price.to_string());

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
                cmd.arg("--full-node");
            }
        }

        if let Some(ref l1_endpoint) = self.l1_endpoint {
            cmd.arg("--l1-endpoint").arg(l1_endpoint);
        } else {
            cmd.arg("--no-l1-sync");
        }

        // Optional arguments
        if let Some(ref chain_config) = self.chain_config_path {
            cmd.arg("--chain-config-path").arg(chain_config);
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
