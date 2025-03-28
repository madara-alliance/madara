#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to parse provider URL: {0}")]
    ProviderUrlParse(#[from] url::ParseError),
    #[error("Failed to parse contract address: {0}")]
    AddressParseError(#[from] alloy::primitives::AddressError),
    #[error("Failed to read config from file: {0}")]
    ConfigReadFromFile(#[from] std::io::Error),
    #[error("Failed to decode from JSON: {0}")]
    ConfigDecodeFromJson(#[from] serde_json::Error),
    #[error("Failed to healthy response from ETH_FORK_URL")]
    RpcNotResponsive,
    #[error("Rate Limits imposed on ETH_FORK_URL")]
    RpcRateLimited,
    #[error("Provided ETH_FORK_URL is invalid format")]
    InvalidForkUrl,
    #[error("No ETH_FORK_URL has been provided")]
    MissingForkUrl,
    #[error("Error spawning anvil instance: {0}")]
    NodeSpawnFailed(#[from] alloy::node_bindings::NodeError),
}
