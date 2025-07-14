use crate::servers::server::ServerError;
use std::path::PathBuf;
use strum_macros::Display;

pub const DEFAULT_ORCHESTRATOR_BINARY: &str = "../target/release/orchestrator";

#[derive(Display, Debug, Clone, PartialEq, Eq)]
pub enum OrchestratorMode {
    #[strum(serialize = "run")]
    Run,
    #[strum(serialize = "setup")]
    Setup,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Layer {
    L2,
    L3,
}

impl std::fmt::Display for Layer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Layer::L2 => write!(f, "l2"),
            Layer::L3 => write!(f, "l3"),
        }
    }
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
}

// Builder type that allows configuration
#[derive(Debug, Clone)]
pub struct OrchestratorConfigBuilder {

    binary_path: PathBuf,

    mode: OrchestratorMode,
    layer: Layer,
    port: Option<u16>,
    environment_vars: Vec<(String, String)>,

    // AWS Configuration
    aws: bool,
    aws_s3: bool,
    aws_sqs: bool,
    aws_sns: bool,
    aws_event_bridge: bool,
    event_bridge_type: Option<String>,

    // Layer-specific options
    settle_on_ethereum: bool,
    settle_on_starknet: bool,
    da_on_ethereum: bool,
    da_on_starknet: bool,
    sharp: bool,
    mongodb: bool,
    atlantic: bool,
}

// Final immutable configuration
#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    binary_path: PathBuf,
    mode: OrchestratorMode,
    layer: Layer,
    port: Option<u16>,
    environment_vars: Vec<(String, String)>,

    // AWS Configuration
    aws: bool,
    aws_s3: bool,
    aws_sqs: bool,
    aws_sns: bool,
    aws_event_bridge: bool,
    event_bridge_type: Option<String>,

    // Layer-specific options
    settle_on_ethereum: bool,
    settle_on_starknet: bool,
    da_on_ethereum: bool,
    da_on_starknet: bool,
    sharp: bool,
    mongodb: bool,
    atlantic: bool,
}

impl Default for OrchestratorConfigBuilder {
    fn default() -> Self {
        Self {
            binary_path: PathBuf::from(DEFAULT_ORCHESTRATOR_BINARY),
            mode: OrchestratorMode::Run,
            layer: Layer::L2,
            port: Some(3000),
            environment_vars: vec![],
            aws: true,
            aws_s3: true,
            aws_sqs: true,
            aws_sns: true,
            aws_event_bridge: true,
            event_bridge_type: Some("rule".to_string()),
            settle_on_ethereum: true,
            settle_on_starknet: false,
            da_on_ethereum: true,
            da_on_starknet: false,
            sharp: false,
            mongodb: true,
            atlantic: false,
        }
    }
}

impl OrchestratorConfigBuilder {
    /// Create a new configuration builder with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the orchestrator mode
    pub fn mode(mut self, mode: OrchestratorMode) -> Self {
        self.mode = mode;
        self
    }

    /// Set the layer (L2 or L3)
    pub fn layer(mut self, layer: Layer) -> Self {
        self.layer = layer;
        self
    }

    /// Set the port
    pub fn port(mut self, port: Option<u16>) -> Self {
        self.port = port;
        self
    }

    /// Add an environment variable
    pub fn add_env_var<K: Into<String>, V: Into<String>>(mut self, key: K, value: V) -> Self {
        self.environment_vars.push((key.into(), value.into()));
        self
    }

    /// Set all environment variables (replaces existing ones)
    pub fn environment_vars(mut self, vars: Vec<(String, String)>) -> Self {
        self.environment_vars = vars;
        self
    }

    /// Enable/disable AWS integration
    pub fn aws(mut self, enabled: bool) -> Self {
        self.aws = enabled;
        self
    }

    /// Enable/disable AWS S3
    pub fn aws_s3(mut self, enabled: bool) -> Self {
        self.aws_s3 = enabled;
        self
    }

    /// Enable/disable AWS SQS
    pub fn aws_sqs(mut self, enabled: bool) -> Self {
        self.aws_sqs = enabled;
        self
    }

    /// Enable/disable AWS SNS
    pub fn aws_sns(mut self, enabled: bool) -> Self {
        self.aws_sns = enabled;
        self
    }

    /// Enable/disable AWS EventBridge
    pub fn aws_event_bridge(mut self, enabled: bool) -> Self {
        self.aws_event_bridge = enabled;
        self
    }

    /// Set the EventBridge type
    pub fn event_bridge_type<S: Into<String>>(mut self, bridge_type: Option<S>) -> Self {
        self.event_bridge_type = bridge_type.map(|s| s.into());
        self
    }

    /// Enable/disable settlement on Ethereum
    pub fn settle_on_ethereum(mut self, enabled: bool) -> Self {
        self.settle_on_ethereum = enabled;
        self
    }

    /// Enable/disable settlement on Starknet
    pub fn settle_on_starknet(mut self, enabled: bool) -> Self {
        self.settle_on_starknet = enabled;
        self
    }

    /// Enable/disable data availability on Ethereum
    pub fn da_on_ethereum(mut self, enabled: bool) -> Self {
        self.da_on_ethereum = enabled;
        self
    }

    /// Enable/disable data availability on Starknet
    pub fn da_on_starknet(mut self, enabled: bool) -> Self {
        self.da_on_starknet = enabled;
        self
    }

    /// Enable/disable SHARP
    pub fn sharp(mut self, enabled: bool) -> Self {
        self.sharp = enabled;
        self
    }

    /// Enable/disable MongoDB
    pub fn mongodb(mut self, enabled: bool) -> Self {
        self.mongodb = enabled;
        self
    }

    /// Enable/disable Atlantic
    pub fn atlantic(mut self, enabled: bool) -> Self {
        self.atlantic = enabled;
        self
    }

    /// Configure for L3 setup with common defaults
    pub fn l3_setup(mut self) -> Self {
        self.layer = Layer::L3;
        self.settle_on_starknet = true;
        self.settle_on_ethereum = false;
        self.da_on_starknet = true;
        self.da_on_ethereum = false;
        self
    }

    /// Configure for Ethereum-based settlement and DA
    pub fn ethereum_stack(mut self) -> Self {
        self.settle_on_ethereum = true;
        self.settle_on_starknet = false;
        self.da_on_ethereum = true;
        self.da_on_starknet = false;
        self
    }

    /// Configure for Starknet-based settlement and DA
    pub fn starknet_stack(mut self) -> Self {
        self.settle_on_ethereum = false;
        self.settle_on_starknet = true;
        self.da_on_ethereum = false;
        self.da_on_starknet = true;
        self
    }

    /// Build the final immutable configuration
    pub fn build(self) -> OrchestratorConfig {
        OrchestratorConfig {
            binary_path: self.binary_path,
            mode: self.mode,
            layer: self.layer,
            port: self.port,
            environment_vars: self.environment_vars,
            aws: self.aws,
            aws_s3: self.aws_s3,
            aws_sqs: self.aws_sqs,
            aws_sns: self.aws_sns,
            aws_event_bridge: self.aws_event_bridge,
            event_bridge_type: self.event_bridge_type,
            settle_on_ethereum: self.settle_on_ethereum,
            settle_on_starknet: self.settle_on_starknet,
            da_on_ethereum: self.da_on_ethereum,
            da_on_starknet: self.da_on_starknet,
            sharp: self.sharp,
            mongodb: self.mongodb,
            atlantic: self.atlantic,
        }
    }
}

impl OrchestratorConfig {
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

    /// Get the environment variables
    pub fn environment_vars(&self) -> &[(String, String)] {
        &self.environment_vars
    }

    /// Check if AWS is enabled
    pub fn aws(&self) -> bool {
        self.aws
    }

    /// Check if AWS S3 is enabled
    pub fn aws_s3(&self) -> bool {
        self.aws_s3
    }

    /// Check if AWS SQS is enabled
    pub fn aws_sqs(&self) -> bool {
        self.aws_sqs
    }

    /// Check if AWS SNS is enabled
    pub fn aws_sns(&self) -> bool {
        self.aws_sns
    }

    /// Check if AWS EventBridge is enabled
    pub fn aws_event_bridge(&self) -> bool {
        self.aws_event_bridge
    }

    /// Get the EventBridge type
    pub fn event_bridge_type(&self) -> Option<&str> {
        self.event_bridge_type.as_deref()
    }

    /// Check if settlement on Ethereum is enabled
    pub fn settle_on_ethereum(&self) -> bool {
        self.settle_on_ethereum
    }

    /// Check if settlement on Starknet is enabled
    pub fn settle_on_starknet(&self) -> bool {
        self.settle_on_starknet
    }

    /// Check if data availability on Ethereum is enabled
    pub fn da_on_ethereum(&self) -> bool {
        self.da_on_ethereum
    }

    /// Check if data availability on Starknet is enabled
    pub fn da_on_starknet(&self) -> bool {
        self.da_on_starknet
    }

    /// Check if SHARP is enabled
    pub fn sharp(&self) -> bool {
        self.sharp
    }

    /// Check if MongoDB is enabled
    pub fn mongodb(&self) -> bool {
        self.mongodb
    }

    /// Check if Atlantic is enabled
    pub fn atlantic(&self) -> bool {
        self.atlantic
    }

    pub fn binary_path(&self) -> &PathBuf {
        &self.binary_path
    }
}
