use crate::servers::server::ServerError;
use std::path::PathBuf;
use strum_macros::Display;
use tokio::process::Command;

pub const DEFAULT_ORCHESTRATOR_BINARY: &str = "../target/release/orchestrator";

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
}

#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    // Runner Config
    binary_path: PathBuf,

    // Orchestrator Config
    mode: OrchestratorMode,
    layer: Layer,
    port: Option<u16>,
    host: String,

    // External Service
    // Database
    mongodb: bool,

    // AWS Configuration
    aws: bool,
    aws_s3: bool,
    aws_sqs: bool,
    aws_sns: bool,
    aws_event_bridge: bool,
    event_bridge_type: AWSEventBridgeType,

    // Layer-specific options

    // Settlement (exclusive)
    settle_on_ethereum: bool,
    settle_on_starknet: bool,

    // Data Availability (exclusive)
    da_on_ethereum: bool,
    da_on_starknet: bool,

    // Prover (exclusive)
    sharp: bool,
    atlantic: bool,

    environment_vars: Vec<(String, String)>,
    additional_args: Vec<String>,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            binary_path: PathBuf::from(DEFAULT_ORCHESTRATOR_BINARY),
            mode: OrchestratorMode::Run,
            layer: Layer::L2,
            port: Some(3000),
            host: "localhost".to_string(),
            additional_args: vec![],
            environment_vars: vec![],
            mongodb: true,
            aws: true,
            aws_s3: true,
            aws_sqs: true,
            aws_sns: true,
            aws_event_bridge: true,
            event_bridge_type: AWSEventBridgeType::Rule,

            settle_on_ethereum: false,
            settle_on_starknet: false,
            da_on_ethereum: false,
            da_on_starknet: false,
            sharp: false,
            atlantic: false,
        }
    }
}

impl OrchestratorConfig {
    /// Create a new configuration with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a builder for OrchestratorConfig
    pub fn builder() -> OrchestratorConfigBuilder {
        OrchestratorConfigBuilder::new()
    }

    // Convenience factory methods for common configurations
    pub fn run_l2() -> Self {
        Self::builder()
            .layer(Layer::L2)
            .mode(OrchestratorMode::Run)
            .atlantic(true)
            .event_bridge_type(AWSEventBridgeType::Rule)
            .settle_on_ethereum(true)
            .da_on_ethereum(true)
            .build()
    }

    pub fn setup_l2() -> Self {
        Self::builder()
            .layer(Layer::L2)
            .mode(OrchestratorMode::Setup)
            .build()
    }

    pub fn run_l3() -> Self {
        Self::builder()
            .layer(Layer::L3)
            .mode(OrchestratorMode::Run)
            .atlantic(true)
            .event_bridge_type(AWSEventBridgeType::Rule)
            .settle_on_starknet(true)
            .da_on_starknet(true)
            .build()
    }

    pub fn setup_l3() -> Self {
        Self::builder()
            .layer(Layer::L3)
            .mode(OrchestratorMode::Setup)
            .build()
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

    /// Get the host
    pub fn host(&self) -> &str {
        &self.host
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

    /// Check if AWS S3 is enabled
    pub fn is_aws_s3_enabled(&self) -> bool {
        self.aws_s3
    }

    /// Check if AWS SQS is enabled
    pub fn is_aws_sqs_enabled(&self) -> bool {
        self.aws_sqs
    }

    /// Check if AWS SNS is enabled
    pub fn is_aws_sns_enabled(&self) -> bool {
        self.aws_sns
    }

    /// Check if AWS EventBridge is enabled
    pub fn is_aws_event_bridge_enabled(&self) -> bool {
        self.aws_event_bridge
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

    /// Check if Atlantic is enabled
    pub fn is_atlantic_enabled(&self) -> bool {
        self.atlantic
    }

    /// Get the binary path
    pub fn binary_path(&self) -> &PathBuf {
        &self.binary_path
    }

    pub fn to_command(&self) -> Command {
        let mut command = Command::new(&self.binary_path);
        command.arg(self.mode.to_string());
        command.arg("--layer").arg(self.layer.to_string());

        if let Some(port) = self.port {
            command.arg("--port").arg(port.to_string());
        }

        command.arg("--host").arg(&self.host);

        // Add database flags
        if self.mongodb {
            command.arg("--mongodb");
        }

        // Add AWS flags
        if self.aws {
            command.arg("--aws");
        }
        if self.aws_s3 {
            command.arg("--aws-s3");
        }
        if self.aws_sqs {
            command.arg("--aws-sqs");
        }
        if self.aws_sns {
            command.arg("--aws-sns");
        }
        if self.aws_event_bridge {
            command.arg("--aws-event-bridge");
        }

        command.arg("--aws-event-bridge-type").arg(self.event_bridge_type.to_string());

        // Add settlement flags
        if self.settle_on_ethereum {
            command.arg("--settle-on-ethereum");
        }
        if self.settle_on_starknet {
            command.arg("--settle-on-starknet");
        }

        // Add data availability flags
        if self.da_on_ethereum {
            command.arg("--da-on-ethereum");
        }
        if self.da_on_starknet {
            command.arg("--da-on-starknet");
        }

        // Add prover flags
        if self.sharp {
            command.arg("--sharp");
        }
        if self.atlantic {
            command.arg("--atlantic");
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
        Self {
            config: OrchestratorConfig::default(),
        }
    }

    /// Build the final configuration
    pub fn build(self) -> OrchestratorConfig {
        self.config
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
    pub fn port(mut self, port: Option<u16>) -> Self {
        self.config.port = port;
        self
    }

    /// Set the host
    pub fn host<S: Into<String>>(mut self, host: S) -> Self {
        self.config.host = host.into();
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

    /// Enable/disable AWS integration
    pub fn aws(mut self, enabled: bool) -> Self {
        self.config.aws = enabled;
        self
    }

    /// Enable/disable AWS S3
    pub fn aws_s3(mut self, enabled: bool) -> Self {
        self.config.aws_s3 = enabled;
        self
    }

    /// Enable/disable AWS SQS
    pub fn aws_sqs(mut self, enabled: bool) -> Self {
        self.config.aws_sqs = enabled;
        self
    }

    /// Enable/disable AWS SNS
    pub fn aws_sns(mut self, enabled: bool) -> Self {
        self.config.aws_sns = enabled;
        self
    }

    /// Enable/disable AWS EventBridge
    pub fn aws_event_bridge(mut self, enabled: bool) -> Self {
        self.config.aws_event_bridge = enabled;
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

    /// Configure for L2 settlement (Ethereum settlement + DA)
    pub fn l2_ethereum_settlement(mut self) -> Self {
        self.config.settle_on_ethereum = true;
        self.config.da_on_ethereum = true;
        self.config.settle_on_starknet = false;
        self.config.da_on_starknet = false;
        self
    }

    /// Configure for L3 settlement (Starknet settlement + DA)
    pub fn l3_starknet_settlement(mut self) -> Self {
        self.config.settle_on_ethereum = false;
        self.config.da_on_ethereum = false;
        self.config.settle_on_starknet = true;
        self.config.da_on_starknet = true;
        self
    }

    /// Enable all AWS services
    pub fn enable_all_aws(mut self) -> Self {
        self.config.aws = true;
        self.config.aws_s3 = true;
        self.config.aws_sqs = true;
        self.config.aws_sns = true;
        self.config.aws_event_bridge = true;
        self
    }

    /// Disable all AWS services
    pub fn disable_all_aws(mut self) -> Self {
        self.config.aws = false;
        self.config.aws_s3 = false;
        self.config.aws_sqs = false;
        self.config.aws_sns = false;
        self.config.aws_event_bridge = false;
        self
    }
}

impl Default for OrchestratorConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}
