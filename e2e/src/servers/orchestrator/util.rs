use crate::servers::server::ServerError;
use std::path::PathBuf;
use strum_macros::Display;

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
}

#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    pub mode: OrchestratorMode,
    pub layer: Layer,
    pub port: Option<u16>,
    pub repository_root: Option<PathBuf>,
    pub environment_vars: Vec<(String, String)>,

    // AWS Configuration
    pub aws: bool,
    pub aws_s3: bool,
    pub aws_sqs: bool,
    pub aws_sns: bool,
    pub aws_event_bridge: bool,
    pub event_bridge_type: Option<String>,

    // Layer-specific options
    pub settle_on_ethereum: bool,
    pub settle_on_starknet: bool,
    pub da_on_ethereum: bool,
    pub da_on_starknet: bool,
    pub sharp: bool,
    pub mongodb: bool,
    pub atlantic: bool,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            mode: OrchestratorMode::Run,
            layer: Layer::L2,
            port: None,
            repository_root: None,
            environment_vars: vec![],
            aws: true,
            aws_s3: true,
            aws_sqs: true,
            aws_sns: true,
            aws_event_bridge: false,
            event_bridge_type: None,
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
