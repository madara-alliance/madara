use crate::services::constants::*;
use crate::services::{helpers::get_binary_path, server::ServerError};
use std::path::PathBuf;
use strum_macros::Display;
use tokio::process::Command;
use url::Url;

// TODO: options are currently limited to per-usages bases

// TODO: might want to re-use these from orchestrator only and not re-write it here!
// Will have to make orchestrator a dependency of e2e then, unsure, ask
#[derive(Display, Debug, Clone, PartialEq, Eq)]
#[strum(serialize_all = "lowercase")]
pub enum OrchestratorMode {
    Run,
    Setup,
}

#[derive(Debug, Clone, PartialEq, Display)]
#[strum(serialize_all = "lowercase")]
pub enum Layer {
    L2,
    L3,
}

#[derive(Debug, Clone, PartialEq, Display)]
#[strum(serialize_all = "lowercase")]
pub enum AWSEventBridgeType {
    Rule,
    Schedule,
}

#[derive(Debug, thiserror::Error)]
pub enum OrchestratorError {
    #[error("Repository root not found")]
    RepositoryRootNotFound,
    #[error("Failed to change working directory: {0}")]
    WorkingDirectoryFailed(std::io::Error),
    #[error("Server error: {0}")]
    Server(#[from] ServerError),
    #[error("Setup mode failed with exit code: {0}")]
    SetupFailed(i32),
    #[error("Missing required dependency: {0}")]
    MissingDependency(String),
    #[error("Orchestrator execution failed: {0}")]
    ExecutionFailed(String),
    #[error("Network error: {0}")]
    NetworkError(String),
    #[error("Invalid response: {0}")]
    InvalidResponse(String),
    #[error("Not Synced yet")]
    NotSynced,
}

#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    // Runner Config
    binary_path: PathBuf,

    // Orchestrator Config
    mode: OrchestratorMode,
    layer: Layer,
    port: Option<u16>,
    logs: (bool, bool),

    // External Service
    // Database
    mongodb: bool,
    mongodb_connection_url: Option<Url>,
    database_name: String,

    // AWS Configuration
    aws: bool,
    event_bridge_type: AWSEventBridgeType,

    // Layer-specific options

    // Settlement (exclusive)
    settle_on_ethereum: bool,
    settle_on_starknet: bool,

    // Data Availability (exclusive)
    da_on_ethereum: bool,
    ethereum_rpc_url: Option<Url>,

    da_on_starknet: bool,

    // Prover (exclusive)
    sharp: bool,
    atlantic: bool,
    atlantic_service_url: Option<Url>,

    // Block Processing
    max_block_to_process: Option<u64>,
    min_block_to_process: Option<u64>,
    madara_version: String,

    environment_vars: Vec<(String, String)>,
    additional_args: Vec<String>,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            binary_path: get_binary_path(ORCHESTRATOR_BINARY),
            mode: OrchestratorMode::Run,
            layer: Layer::L2,
            port: None,
            additional_args: vec![],
            environment_vars: vec![],
            mongodb: true,
            mongodb_connection_url: Some(
                Url::parse(format!("mongodb://{}:{}", DEFAULT_SERVICE_HOST, MONGODB_PORT).as_str()).unwrap(),
            ),
            aws: true,
            event_bridge_type: AWSEventBridgeType::Rule,
            database_name: String::from(ORCHESTRATOR_DATABASE_NAME),
            settle_on_ethereum: false,
            settle_on_starknet: false,
            da_on_ethereum: false,
            ethereum_rpc_url: None,
            da_on_starknet: false,
            sharp: false,
            atlantic: false,
            atlantic_service_url: None,

            max_block_to_process: None,
            min_block_to_process: None,
            madara_version: "0.14.0".to_string(),
            logs: (false, true),
        }
    }
}

impl OrchestratorConfig {
    /// Create a new configuration with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a builder for OrchestratorConfig from the current state
    pub fn builder(self) -> OrchestratorConfigBuilder {
        OrchestratorConfigBuilder { config: self }
    }

    // Getter methods (immutable access)

    /// Get the orchestrator mode
    pub fn mode(&self) -> &OrchestratorMode {
        &self.mode
    }

    /// Get the layer
    pub fn layer(&self) -> &Layer {
        &self.layer
    }

    /// Get the port
    pub fn port(&self) -> Option<u16> {
        self.port
    }

    /// Get the logs
    pub fn logs(&self) -> (bool, bool) {
        self.logs
    }

    /// Get the environment variables
    pub fn environment_vars(&self) -> &[(String, String)] {
        &self.environment_vars
    }

    /// Get additional arguments
    pub fn additional_args(&self) -> &[String] {
        &self.additional_args
    }

    /// Check if AWS is enabled
    pub fn is_aws_enabled(&self) -> bool {
        self.aws
    }

    /// Get the EventBridge type
    pub fn event_bridge_type(&self) -> &AWSEventBridgeType {
        &self.event_bridge_type
    }

    /// Check if settlement on Ethereum is enabled
    pub fn is_settle_on_ethereum_enabled(&self) -> bool {
        self.settle_on_ethereum
    }

    /// Check if settlement on Starknet is enabled
    pub fn is_settle_on_starknet_enabled(&self) -> bool {
        self.settle_on_starknet
    }

    /// Check if data availability on Ethereum is enabled
    pub fn is_da_on_ethereum_enabled(&self) -> bool {
        self.da_on_ethereum
    }

    /// Get the Ethereum data availability RPC URL
    pub fn ethereum_rpc_url(&self) -> Option<&Url> {
        self.ethereum_rpc_url.as_ref()
    }

    /// Check if data availability on Starknet is enabled
    pub fn is_da_on_starknet_enabled(&self) -> bool {
        self.da_on_starknet
    }

    /// Check if SHARP is enabled
    pub fn is_sharp_enabled(&self) -> bool {
        self.sharp
    }

    /// Check if MongoDB is enabled
    pub fn is_mongodb_enabled(&self) -> bool {
        self.mongodb
    }

    /// Get the MongoDB connection string
    pub fn mongodb_connection_url(&self) -> Option<&Url> {
        self.mongodb_connection_url.as_ref()
    }

    /// Check if Atlantic is enabled
    pub fn is_atlantic_enabled(&self) -> bool {
        self.atlantic
    }

    /// Get the Atlantic service URL
    pub fn atlantic_service_url(&self) -> Option<&Url> {
        self.atlantic_service_url.as_ref()
    }

    /// Get the binary path
    pub fn binary_path(&self) -> &PathBuf {
        &self.binary_path
    }

    /// Get the database name
    pub fn database_name(&self) -> &str {
        &self.database_name
    }

    pub fn to_command(&self) -> Command {
        let mut command = Command::new(&self.binary_path);
        command.arg(self.mode.to_string());
        command.arg("--layer").arg(self.layer.to_string());

        // Add AWS flags (Needed in both)
        if self.aws {
            command.arg("--aws");
            command.arg("--aws-s3");
            command.arg("--aws-sqs");
            command.arg("--aws-sns");
        }

        if *self.mode() == OrchestratorMode::Run {
            command = self.to_command_run(command);
        } else {
            command = self.to_command_setup(command);
        }

        command
    }

    pub fn to_command_setup(&self, mut command: Command) -> Command {
        command.arg("--aws-event-bridge");
        command.arg("--event-bridge-type").arg(self.event_bridge_type.to_string());

        // Add environment variables
        for (key, value) in &self.environment_vars {
            command.env(key, value);
        }

        // Add additional arguments
        for arg in &self.additional_args {
            command.arg(arg);
        }

        command
    }

    pub fn to_command_run(&self, mut command: Command) -> Command {
        if let Some(port) = self.port {
            command.arg("--port").arg(port.to_string());
        }

        // TODO: might wanna remove it ?
        if self.mongodb {
            command.arg("--mongodb");
            command.arg("--mongodb-database-name").arg(self.database_name());
            if let Some(connection_url) = self.mongodb_connection_url() {
                command.arg("--mongodb-connection-url").arg(connection_url.as_str());
            }
        }

        // Add settlement flags
        if self.settle_on_ethereum {
            command.arg("--settle-on-ethereum");
            command.arg("--ethereum-finality-retry-wait-in-secs").arg(4.to_string());
            if let Some(rpc_url) = self.ethereum_rpc_url() {
                command.arg("--ethereum-rpc-url").arg(rpc_url.to_string());
            }
        }
        if self.settle_on_starknet {
            command.arg("--settle-on-starknet");
        }

        // Add data availability flags
        if self.da_on_ethereum {
            command.arg("--da-on-ethereum");
            if let Some(rpc_url) = self.ethereum_rpc_url() {
                command.arg("--ethereum-da-rpc-url").arg(rpc_url.to_string());
            }
        }
        if self.da_on_starknet {
            command.arg("--da-on-starknet");
        }

        command.arg("--madara-version").arg(&self.madara_version);
        command.arg("--disable-peerdas");

        // Add prover flags
        if self.sharp {
            command.arg("--sharp");
        }
        if self.atlantic {
            command.arg("--atlantic");
            if let Some(service_url) = self.atlantic_service_url() {
                command.arg("--atlantic-service-url").arg(service_url.to_string());
            }
        }

        if let Some(max_block) = self.max_block_to_process {
            command.arg("--max-block-to-process").arg(max_block.to_string());
        }
        if let Some(min_block) = self.min_block_to_process {
            command.arg("--min-block-to-process").arg(min_block.to_string());
        }

        // Add environment variables
        for (key, value) in &self.environment_vars {
            command.env(key, value);
        }

        // Add additional arguments
        for arg in &self.additional_args {
            command.arg(arg);
        }

        command
    }
}

/// Builder for OrchestratorConfig
#[derive(Debug, Clone)]
pub struct OrchestratorConfigBuilder {
    config: OrchestratorConfig,
}

impl OrchestratorConfigBuilder {
    /// Create a new builder with default configuration
    pub fn new() -> Self {
        Self { config: OrchestratorConfig::default() }
    }

    /// Build the final configuration
    pub fn build(self) -> OrchestratorConfig {
        self.config
    }

    // Convenience factory methods for common configurations - now return builder for chaining
    pub fn run_l2() -> Self {
        Self::new()
            .layer(Layer::L2)
            .mode(OrchestratorMode::Run)
            .atlantic(true)
            .event_bridge_type(AWSEventBridgeType::Rule)
            .settle_on_ethereum(true)
            .da_on_ethereum(true)
    }

    pub fn setup_l2() -> Self {
        Self::new().layer(Layer::L2).mode(OrchestratorMode::Setup)
    }

    pub fn run_l3() -> Self {
        Self::new()
            .layer(Layer::L3)
            .mode(OrchestratorMode::Run)
            .atlantic(true)
            .event_bridge_type(AWSEventBridgeType::Rule)
            .settle_on_starknet(true)
            .da_on_starknet(true)
    }

    pub fn setup_l3() -> Self {
        Self::new().layer(Layer::L3).mode(OrchestratorMode::Setup)
    }

    /// Set the binary path
    pub fn binary_path<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.config.binary_path = path.into();
        self
    }

    /// Set the orchestrator mode
    pub fn mode(mut self, mode: OrchestratorMode) -> Self {
        self.config.mode = mode;
        self
    }

    /// Set the layer (L2 or L3)
    pub fn layer(mut self, layer: Layer) -> Self {
        self.config.layer = layer;
        self
    }

    /// Set the port
    pub fn port(mut self, port: u16) -> Self {
        self.config.port = Some(port);
        self
    }

    pub fn madara_version(mut self, version: &str) -> Self {
        self.config.madara_version = version.to_string();
        self
    }

    pub fn max_block_to_process(mut self, block_number: u64) -> Self {
        self.config.max_block_to_process = Some(block_number);
        self
    }

    pub fn min_block_to_process(mut self, block_number: u64) -> Self {
        self.config.min_block_to_process = Some(block_number);
        self
    }

    /// Add an environment variable
    pub fn env_var<K: Into<String>, V: Into<String>>(mut self, key: K, value: V) -> Self {
        self.config.environment_vars.push((key.into(), value.into()));
        self
    }

    /// Set all environment variables (replaces existing ones)
    pub fn environment_vars(mut self, vars: Vec<(String, String)>) -> Self {
        self.config.environment_vars = vars;
        self
    }

    /// Add an additional argument
    pub fn arg<S: Into<String>>(mut self, arg: S) -> Self {
        self.config.additional_args.push(arg.into());
        self
    }

    /// Set all additional arguments (replaces existing ones)
    pub fn additional_args(mut self, args: Vec<String>) -> Self {
        self.config.additional_args = args;
        self
    }

    /// Enable/disable MongoDB
    pub fn mongodb(mut self, enabled: bool) -> Self {
        self.config.mongodb = enabled;
        self
    }

    /// Set the MongoDB connection URL
    pub fn mongodb_connection_url(mut self, url: Url) -> Self {
        self.config.mongodb_connection_url = Some(url);
        self
    }

    /// Enable/disable AWS integration
    pub fn aws(mut self, enabled: bool) -> Self {
        self.config.aws = enabled;
        self
    }

    /// Set the EventBridge type
    pub fn event_bridge_type(mut self, event_bridge_type: AWSEventBridgeType) -> Self {
        self.config.event_bridge_type = event_bridge_type;
        self
    }

    /// Enable/disable settlement on Ethereum
    pub fn settle_on_ethereum(mut self, enabled: bool) -> Self {
        self.config.settle_on_ethereum = enabled;
        self
    }

    /// Set the Ethereum data availability RPC URL
    pub fn ethereum_rpc_url(mut self, url: Url) -> Self {
        self.config.ethereum_rpc_url = Some(url);
        self
    }

    /// Enable/disable settlement on Starknet
    pub fn settle_on_starknet(mut self, enabled: bool) -> Self {
        self.config.settle_on_starknet = enabled;
        self
    }

    /// Enable/disable data availability on Ethereum
    pub fn da_on_ethereum(mut self, enabled: bool) -> Self {
        self.config.da_on_ethereum = enabled;
        self
    }

    /// Enable/disable data availability on Starknet
    pub fn da_on_starknet(mut self, enabled: bool) -> Self {
        self.config.da_on_starknet = enabled;
        self
    }

    /// Enable/disable SHARP prover
    pub fn sharp(mut self, enabled: bool) -> Self {
        self.config.sharp = enabled;
        self
    }

    /// Enable/disable Atlantic prover
    pub fn atlantic(mut self, enabled: bool) -> Self {
        self.config.atlantic = enabled;
        self
    }

    /// Set the Atlantic service URL
    pub fn atlantic_service_url(mut self, url: Url) -> Self {
        self.config.atlantic_service_url = Some(url);
        self
    }

    /// Set the logs
    pub fn logs(mut self, logs: (bool, bool)) -> Self {
        self.config.logs = logs;
        self
    }
}

impl Default for OrchestratorConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}
