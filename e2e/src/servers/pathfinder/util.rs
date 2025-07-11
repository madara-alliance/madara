const DEFAULT_PATHFINDER_PORT: u16 = 9545;
const DEFAULT_PATHFINDER_IMAGE: &str = "eqlabs/pathfinder:v0.17.0-beta.2";
const DEFAULT_PATHFINDER_CONTAINER_NAME: &str = "pathfinder-service";
const DEFAULT_PATHFINDER_MONITOR_PORT: u16 = 9090;

use crate::servers::docker::DockerError;

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
}

#[derive(Debug, Clone)]
pub struct PathfinderConfig {
    pub port: u16,
    pub monitor_port: u16,
    pub image: String,
    pub container_name: String,
    pub ethereum_url: String,
    pub data_directory: String,
    pub rpc_root_version: String,
    pub network: String,
    pub chain_id: String,
    pub gateway_url: Option<String>,
    pub feeder_gateway_url: Option<String>,
    pub storage_state_tries: String,
    pub gateway_request_timeout: u64,
    pub data_volume: Option<String>, // For persistent data
    pub environment_vars: Vec<(String, String)>,
}

impl Default for PathfinderConfig {
    fn default() -> Self {
        Self {
            port: DEFAULT_PATHFINDER_PORT,
            monitor_port: DEFAULT_PATHFINDER_MONITOR_PORT,
            image: DEFAULT_PATHFINDER_IMAGE.to_string(),
            container_name: DEFAULT_PATHFINDER_CONTAINER_NAME.to_string(),
            ethereum_url: "https://ethereum-sepolia-rpc.publicnode.com".to_string(),
            data_directory: "/var/pathfinder".to_string(),
            rpc_root_version: "v07".to_string(),
            network: "custom".to_string(),
            chain_id: "MADARA_DEVNET".to_string(),
            gateway_url: Some("http://host.docker.internal:9943/feeder".to_string()),
            feeder_gateway_url: Some("http://host.docker.internal:9943/feeder_gateway".to_string()),
            storage_state_tries: "archive".to_string(),
            gateway_request_timeout: 1000,
            data_volume: None,
            environment_vars: vec![],
        }
    }
}
