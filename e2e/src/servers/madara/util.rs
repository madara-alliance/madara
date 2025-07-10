use crate::servers::server::ServerError;
use std::collections::HashMap;
use std::path::PathBuf;

const DEFAULT_MADARA_RPC_PORT: u16 = 9944;
const DEFAULT_MADARA_GATEWAY_PORT: u16 = 8080;
const DEFAULT_MADARA_NAME: &str = "madara";
pub const DEFAULT_MADARA_BINARY: &str = "madara";

#[derive(Debug, thiserror::Error)]
pub enum MadaraError {
    #[error("Madara binary not found: {0}")]
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
}

#[derive(Debug, Clone)]
pub struct MadaraConfig {
    pub name: String,
    pub database_path: PathBuf,
    pub rpc_port: u16,
    pub gateway_port: u16,
    pub rpc_cors: String,
    pub rpc_external: bool,
    pub rpc_admin: bool,
    pub sequencer: bool,
    pub chain_config_path: Option<PathBuf>,
    pub feeder_gateway_enable: bool,
    pub gateway_enable: bool,
    pub gateway_external: bool,
    pub l1_endpoint: String,
    pub strk_gas_price: u64,
    pub strk_blob_gas_price: u64,
    pub gas_price: u64,
    pub blob_gas_price: u64,
    pub binary_path: Option<PathBuf>,
    pub environment_vars: HashMap<String, String>,
    pub additional_args: Vec<String>,
    pub release_mode: bool,
}

impl Default for MadaraConfig {
    fn default() -> Self {
        Self {
            name: DEFAULT_MADARA_NAME.to_string(),
            database_path: PathBuf::from("../madara-db"),
            rpc_port: DEFAULT_MADARA_RPC_PORT,
            gateway_port: DEFAULT_MADARA_GATEWAY_PORT,
            rpc_cors: "*".to_string(),
            rpc_external: true,
            rpc_admin: true,
            sequencer: true,
            chain_config_path: Some(PathBuf::from("../configs/presets/devnet.yaml")),
            feeder_gateway_enable: true,
            gateway_enable: true,
            gateway_external: true,
            l1_endpoint: "http://127.0.0.1:8545".to_string(),
            strk_gas_price: 0,
            strk_blob_gas_price: 0,
            gas_price: 0,
            blob_gas_price: 0,
            // ./target/release/madara
            binary_path: Some(PathBuf::from("../target/release/madara")),
            environment_vars: HashMap::new(),
            additional_args: Vec::new(),
            release_mode: false,
        }
    }
}

pub struct MadaraCMDBuilder {
    args: Vec<String>,
    env: HashMap<String, String>,
}

impl MadaraCMDBuilder {
    pub fn new() -> Self {
        Self { args: Vec::new(), env: HashMap::new() }
    }

    pub fn with_config(config: &MadaraConfig) -> Self {
        let mut builder = Self::new();
        builder.build_from_config(config);
        builder
    }

    fn build_from_config(&mut self, config: &MadaraConfig) {
        // Core arguments
        self.add_arg("--name", &config.name);
        self.add_arg("--base-path", config.database_path.to_string_lossy().as_ref());
        self.add_arg("--rpc-port", &config.rpc_port.to_string());
        self.add_arg("--rpc-cors", &config.rpc_cors);
        self.add_arg("--gateway-port", &config.gateway_port.to_string());
        self.add_flag("--no-l1-sync");
        self.add_flag("--no-charge-fee");
        self.add_arg("--strk-gas-price", &config.strk_gas_price.to_string());
        self.add_arg("--strk-blob-gas-price", &config.strk_blob_gas_price.to_string());
        self.add_arg("--gas-price", &config.gas_price.to_string());
        self.add_arg("--blob-gas-price", &config.blob_gas_price.to_string());

        // Boolean flags
        if config.rpc_external {
            self.add_flag("--rpc-external");
        }
        if config.rpc_admin {
            self.add_flag("--rpc-admin");
        }
        if config.sequencer {
            self.add_flag("--sequencer");
        }
        if config.feeder_gateway_enable {
            self.add_flag("--feeder-gateway-enable");
        }
        if config.gateway_enable {
            self.add_flag("--gateway-enable");
        }
        if config.gateway_external {
            self.add_flag("--gateway-external");
        }

        // Optional arguments
        if let Some(ref chain_config) = config.chain_config_path {
            self.add_arg("--chain-config-path", chain_config.to_string_lossy().as_ref());
        }

        // Additional arguments
        for arg in &config.additional_args {
            self.args.push(arg.clone());
        }

        // Environment variables
        for (key, value) in &config.environment_vars {
            self.env.insert(key.clone(), value.clone());
        }
    }

    pub fn add_arg(&mut self, key: &str, value: &str) -> &mut Self {
        self.args.push(key.to_string());
        self.args.push(value.to_string());
        self
    }

    pub fn add_flag(&mut self, flag: &str) -> &mut Self {
        self.args.push(flag.to_string());
        self
    }

    pub fn add_env(&mut self, key: &str, value: &str) -> &mut Self {
        self.env.insert(key.to_string(), value.to_string());
        self
    }

    pub fn build(&self) -> MadaraCMD {
        MadaraCMD { args: self.args.clone(), env: self.env.clone() }
    }
}

#[derive(Debug, Clone)]
pub struct MadaraCMD {
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

impl MadaraCMD {
    pub fn new(args: Vec<String>, env: HashMap<String, String>) -> Self {
        Self { args, env }
    }

    pub fn from_config(config: &MadaraConfig) -> Self {
        MadaraCMDBuilder::with_config(config).build()
    }
}
