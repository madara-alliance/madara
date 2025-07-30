use crate::services::helpers::get_container_name;
use crate::services::server::ServerError;
use tokio::process::Command;
use url::Url;

#[derive(Debug, thiserror::Error)]
pub enum LocalstackError {
    #[error("Docker error: {0}")]
    Docker(#[from] DockerError),
    #[error("Localstack container already running on port {0}")]
    AlreadyRunning(u16),
    #[error("Port {0} is already in use")]
    PortInUse(u16),
    #[error("Server error: {0}")]
    Server(#[from] ServerError),
}

use crate::services::constants::*;
use crate::services::docker::DockerError;

// Final immutable configuration
#[derive(Debug, Clone)]
pub struct LocalstackConfig {
    image: String,
    container_name: String,
    environment_vars: Vec<(String, String)>,

    // Server Configs
    port: u16,
    logs: (bool, bool),
}

impl Default for LocalstackConfig {
    fn default() -> Self {
        Self {
            image: LOCALSTACK_IMAGE.to_string(),
            container_name: get_container_name(LOCALSTACK_CONTAINER),
            environment_vars: vec![("SERVICES".to_string(), "iam,s3,eventbridge,events,sqs,sns".to_string())],

            port: LOCALSTACK_PORT,
            logs: (false, true),
        }
    }
}

impl LocalstackConfig {
    /// Create a new configuration with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a builder for LocalstackConfig
    pub fn builder() -> LocalstackConfigBuilder {
        LocalstackConfigBuilder::new()
    }

    /// Get the port
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Get the logs
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

    /// Get the environment variables
    pub fn environment_vars(&self) -> &[(String, String)] {
        &self.environment_vars
    }

    /// Get the endpoint URL
    pub fn endpoint(&self) -> Url {
        let url = format!("http://{}:{}", DEFAULT_SERVICE_HOST, self.port());
        Url::parse(&url).unwrap()
    }

    /// Build the Docker command for Localstack
    pub fn to_command(&self) -> Command {
        let mut command = Command::new("docker");
        command.arg("run");
        command.arg("--rm"); // Remove container when it stops
        command.arg("--name").arg(self.container_name());
        command.arg("-p").arg(format!("{}:{}", self.port(), LOCALSTACK_PORT));

        // Add environment variables
        for (key, value) in self.environment_vars() {
            command.arg("-e").arg(format!("{}={}", key, value));
        }

        command.arg(self.image());

        command
    }
}

// Builder type that allows configuration
#[derive(Debug, Clone)]
pub struct LocalstackConfigBuilder {
    config: LocalstackConfig,
}

impl LocalstackConfigBuilder {
    /// Create a new configuration builder with default values
    pub fn new() -> Self {
        Self { config: LocalstackConfig::default() }
    }

    /// Build the final immutable configuration
    pub fn build(self) -> LocalstackConfig {
        self.config
    }

    /// Set the port (default: 4566)
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

    /// Add an environment variable
    pub fn env_var<K: Into<String>, V: Into<String>>(mut self, key: K, value: V) -> Self {
        self.config.environment_vars.push((key.into(), value.into()));
        self
    }

    /// Set the logs
    pub fn logs(mut self, logs: (bool, bool)) -> Self {
        self.config.logs = logs;
        self
    }

    /// Set all environment variables (replaces existing ones)
    pub fn environment_vars(mut self, vars: Vec<(String, String)>) -> Self {
        self.config.environment_vars = vars;
        self
    }
}

impl Default for LocalstackConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}
