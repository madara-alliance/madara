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
}
