use crate::servers::docker::DockerError;

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
}

// Builder type that allows configuration
#[derive(Debug, Clone)]
pub struct MongoConfigBuilder {
    port: u16,
    image: String,
    container_name: String,
}

// Final immutable configuration
#[derive(Debug, Clone)]
pub struct MongoConfig {
    port: u16,
    image: String,
    container_name: String,
}

impl Default for MongoConfigBuilder {
    fn default() -> Self {
        Self {
            port: DEFAULT_MONGO_PORT,
            image: DEFAULT_MONGO_IMAGE.to_string(),
            container_name: DEFAULT_MONGO_CONTAINER_NAME.to_string(),
        }
    }
}

impl MongoConfigBuilder {
    /// Create a new configuration builder with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the port (default: 27017)
    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Set the Docker image
    pub fn image<S: Into<String>>(mut self, image: S) -> Self {
        self.image = image.into();
        self
    }

    /// Set the container name
    pub fn container_name<S: Into<String>>(mut self, name: S) -> Self {
        self.container_name = name.into();
        self
    }

    /// Build the final immutable configuration
    pub fn build(self) -> MongoConfig {
        MongoConfig {
            port: self.port,
            image: self.image,
            container_name: self.container_name,
        }
    }
}

impl MongoConfig {
    /// Get the port
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Get the Docker image
    pub fn image(&self) -> &str {
        &self.image
    }

    /// Get the container name
    pub fn container_name(&self) -> &str {
        &self.container_name
    }
}
