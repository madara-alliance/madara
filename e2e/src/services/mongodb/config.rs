use crate::services::docker::DockerError;
use tokio::process::Command;
use crate::services::server::ServerError;

const DEFAULT_MONGO_PORT: u16 = 27017;
pub const DEFAULT_MONGO_IMAGE: &str = "mongo:latest";
const DEFAULT_MONGO_CONTAINER_NAME: &str = "mongodb-service";
pub const MONGO_DEFAULT_DATABASE_PATH: &str = "mongodb_dump.json";

#[derive(Debug, thiserror::Error)]
pub enum MongoError {
    #[error("Docker error: {0}")]
    Docker(#[from] DockerError),
    #[error("MongoDB container already running on port {0}")]
    AlreadyRunning(u16),
    #[error("Port {0} is already in use")]
    PortInUse(u16),
    #[error("MongoDB connection failed: {0}")]
    ConnectionFailed(String),
    #[error("Server error: {0}")]
    Server(#[from] ServerError),
}

// Final immutable configuration
#[derive(Debug, Clone)]
pub struct MongoConfig {
    image: String,
    container_name: String,

    // Server configs
    port: u16,
    logs: (bool,bool),
}

impl Default for MongoConfig {
    fn default() -> Self {
        Self {
            image: DEFAULT_MONGO_IMAGE.to_string(),
            container_name: DEFAULT_MONGO_CONTAINER_NAME.to_string(),
            port: DEFAULT_MONGO_PORT,
            logs: (true,true),
        }
    }
}

impl MongoConfig {
    /// Create a new configuration with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a builder for MongoConfig
    pub fn builder() -> MongoConfigBuilder {
        MongoConfigBuilder::new()
    }

    /// Get the port
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Get the logs
    pub fn logs(&self) -> (bool,bool) {
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


    /// Build the Docker command for MongoDB
    pub fn to_command(&self) -> Command {
        let mut command = Command::new("docker");
        command.arg("run");
        command.arg("--rm"); // Remove container when it stops
        command.arg("--name").arg(self.container_name());
        command.arg("-p").arg(format!("{}:27017", self.port()));
        command.arg(self.image());

        command
    }



}

// Builder type that allows configuration
#[derive(Debug, Clone)]
pub struct MongoConfigBuilder {
    config: MongoConfig,
}

impl MongoConfigBuilder {
    /// Create a new configuration builder with default values
    pub fn new() -> Self {
        Self {
            config: MongoConfig::default(),
        }
    }

    /// Set the port (default: 27017)
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

    /// Build the final immutable configuration
    pub fn build(self) -> MongoConfig {
        self.config
    }
}

impl Default for MongoConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}
