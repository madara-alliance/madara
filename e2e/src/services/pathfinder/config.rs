// Originally implemented with just a struct with public fields. We've refactored
// to use a builder pattern that ensures immutability after building while providing
// a clean, fluent API for configuration.

use tokio::process::Command;
use crate::services::helpers::NodeRpcError;

use crate::services::docker::DockerError;
use crate::services::server::ServerError;

const DEFAULT_PATHFINDER_PORT: u16 = 9545;
pub const DEFAULT_PATHFINDER_IMAGE: &str = "prkpandey942/pathfinder:549aa84_2025-05-29_appchain-vers-cons_amd";
const DEFAULT_PATHFINDER_CONTAINER_NAME: &str = "pathfinder-service";

#[derive(Debug, thiserror::Error)]
pub enum PathfinderError {
    #[error("Docker error: {0}")]
    Docker(#[from] DockerError),
    #[error("Pathfinder container already running on port {0}")]
    AlreadyRunning(u16),
    #[error("Port {0} is already in use")]
    PortInUse(u16),
    #[error("Pathfinder connection failed: {0}")]
    ConnectionFailed(String),
    #[error("Missing required configuration: {0}")]
    MissingConfig(String),
    #[error("Invalid response from Pathfinder")]
    InvalidResponse,
    #[error("RPC error: {0}")]
    RpcError(#[from] NodeRpcError),
    #[error("Server error: {0}")]
    Server(#[from] ServerError),
}

// Final immutable configuration
#[derive(Debug, Clone)]
pub struct PathfinderConfig {
    port: u16,
    image: String,
    container_name: String,
    ethereum_url: String,
    data_directory: String,
    rpc_root_version: String,
    network: String,
    chain_id: String,
    gateway_url: Option<String>,
    feeder_gateway_url: Option<String>,
    storage_state_tries: String,
    gateway_request_timeout: u64,
    data_volume: Option<String>,
    logs: (bool, bool),
    environment_vars: Vec<(String, String)>,
}

impl Default for PathfinderConfig {
    fn default() -> Self {
        Self {
            port: DEFAULT_PATHFINDER_PORT,
            image: DEFAULT_PATHFINDER_IMAGE.to_string(),
            container_name: format!("{}-{}", DEFAULT_PATHFINDER_CONTAINER_NAME, uuid::Uuid::new_v4()),
            ethereum_url: "https://ethereum-sepolia-rpc.publicnode.com".to_string(),
            data_directory: "/var/pathfinder".to_string(),
            rpc_root_version: "v07".to_string(),
            network: "custom".to_string(),
            chain_id: "MADARA_DEVNET".to_string(),
            gateway_url: Some("http://host.docker.internal:8080/feeder".to_string()),
            feeder_gateway_url: Some("http://host.docker.internal:8080/feeder_gateway".to_string()),
            storage_state_tries: "archive".to_string(),
            gateway_request_timeout: 1000,
            data_volume: None,
            environment_vars: vec![],
            logs: (true, true),
        }
    }
}

impl PathfinderConfig {
    /// Create a new configuration with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a builder for PathfinderConfig
    pub fn builder() -> PathfinderConfigBuilder {
        PathfinderConfigBuilder::new()
    }

    /// Get the RPC port
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Get the logs configuration
    pub fn logs(&self) -> (bool, bool) {
        self.logs
    }

    /// Get the Docker image
    pub fn image(&self) -> &str {
        &self.image
    }

    /// Get the container name
    pub fn container_name(&self) -> &str {
        &self.container_name
    }

    /// Get the Ethereum URL
    pub fn ethereum_url(&self) -> &str {
        &self.ethereum_url
    }

    /// Get the data directory
    pub fn data_directory(&self) -> &str {
        &self.data_directory
    }

    /// Get the RPC root version
    pub fn rpc_root_version(&self) -> &str {
        &self.rpc_root_version
    }

    /// Get the network
    pub fn network(&self) -> &str {
        &self.network
    }

    /// Get the chain ID
    pub fn chain_id(&self) -> &str {
        &self.chain_id
    }

    /// Get the gateway URL
    pub fn gateway_url(&self) -> Option<&str> {
        self.gateway_url.as_deref()
    }

    /// Get the feeder gateway URL
    pub fn feeder_gateway_url(&self) -> Option<&str> {
        self.feeder_gateway_url.as_deref()
    }

    /// Get the storage state tries
    pub fn storage_state_tries(&self) -> &str {
        &self.storage_state_tries
    }

    /// Get the gateway request timeout
    pub fn gateway_request_timeout(&self) -> u64 {
        self.gateway_request_timeout
    }

    /// Get the data volume
    pub fn data_volume(&self) -> Option<&str> {
        self.data_volume.as_deref()
    }

    /// Get the environment variables
    pub fn environment_vars(&self) -> &[(String, String)] {
        &self.environment_vars
    }

    pub fn to_command(&self) -> Command {
        let mut command = Command::new("docker");
        command.arg("run");
        command.arg("--rm"); // Remove container when it stops
        command.arg("--name").arg(self.container_name());

        // Port mappings
        command.arg("-p").arg(format!("{}:{}", self.port(), self.port()));

        // Add data volume if specified
        if let Some(volume) = self.data_volume() {
            command.arg("-v").arg(format!("{}:{}", volume, self.data_directory()));
        }

        // Add custom environment variables
        for (key, value) in self.environment_vars() {
            command.arg("-e").arg(format!("{}={}", key, value));
        }

        // Add the image
        command.arg(self.image());

        // Add pathfinder binary command and arguments
        command.arg("--ethereum.url").arg(self.ethereum_url());
        // command.arg("--data-directory").arg(config.data_directory());
        command.arg("--http-rpc").arg(format!("0.0.0.0:{}", self.port()));
        command.arg("--rpc.root-version").arg(self.rpc_root_version());
        command.arg("--network").arg(self.network());
        command.arg("--chain-id").arg(self.chain_id());

        if let Some(gateway_url) = self.gateway_url() {
            // command.arg("--add-host");
            command.arg("--gateway-url").arg(gateway_url);
        }

        if let Some(feeder_gateway_url) = self.feeder_gateway_url() {
            // command.arg("--add-host");
            command.arg("--feeder-gateway-url").arg(feeder_gateway_url);
        }

        command.arg("--storage.state-tries").arg(self.storage_state_tries());
        command.arg("--gateway.request-timeout").arg(self.gateway_request_timeout().to_string());

        command
    }

}

// Builder type that allows configuration
#[derive(Debug, Clone)]
pub struct PathfinderConfigBuilder {
    config: PathfinderConfig,
}

impl PathfinderConfigBuilder {
    /// Create a new configuration builder with default values
    pub fn new() -> Self {
        Self {
            config: PathfinderConfig::default(),
        }
    }

    /// Build the final immutable configuration
    pub fn build(self) -> PathfinderConfig {
        self.config
    }

    /// Set the RPC port (default: 9545)
    pub fn port(mut self, port: u16) -> Self {
        self.config.port = port;
        self
    }

    /// Set the Docker image
    pub fn image<S: Into<String>>(mut self, image: S) -> Self {
        self.config.image = image.into();
        self
    }

    /// Set the container name
    pub fn container_name<S: Into<String>>(mut self, name: S) -> Self {
        self.config.container_name = name.into();
        self
    }

    /// Set the Ethereum URL
    pub fn ethereum_url<S: Into<String>>(mut self, url: S) -> Self {
        self.config.ethereum_url = url.into();
        self
    }

    /// Set the data directory
    pub fn data_directory<S: Into<String>>(mut self, directory: S) -> Self {
        self.config.data_directory = directory.into();
        self
    }

    /// Set the RPC root version
    pub fn rpc_root_version<S: Into<String>>(mut self, version: S) -> Self {
        self.config.rpc_root_version = version.into();
        self
    }

    /// Set the network
    pub fn network<S: Into<String>>(mut self, network: S) -> Self {
        self.config.network = network.into();
        self
    }

    /// Set the chain ID
    pub fn chain_id<S: Into<String>>(mut self, chain_id: S) -> Self {
        self.config.chain_id = chain_id.into();
        self
    }

    /// Set the gateway URL
    pub fn gateway_url<S: Into<String>>(mut self, url: Option<S>) -> Self {
        self.config.gateway_url = url.map(|u| u.into());
        self
    }

    /// Set the feeder gateway URL
    pub fn feeder_gateway_url<S: Into<String>>(mut self, url: Option<S>) -> Self {
        self.config.feeder_gateway_url = url.map(|u| u.into());
        self
    }

    /// Set the storage state tries
    pub fn storage_state_tries<S: Into<String>>(mut self, tries: S) -> Self {
        self.config.storage_state_tries = tries.into();
        self
    }

    /// Set the gateway request timeout
    pub fn gateway_request_timeout(mut self, timeout: u64) -> Self {
        self.config.gateway_request_timeout = timeout;
        self
    }

    /// Set the data volume for persistent storage
    pub fn data_volume<S: Into<String>>(mut self, volume: Option<S>) -> Self {
        self.config.data_volume = volume.map(|v| v.into());
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

    /// Clear all environment variables
    pub fn clear_env_vars(mut self) -> Self {
        self.config.environment_vars.clear();
        self
    }
}

impl Default for PathfinderConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}
