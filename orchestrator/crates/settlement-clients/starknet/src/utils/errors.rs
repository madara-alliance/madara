#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    SerdeJson(#[from] serde_json::Error),
    #[error("['bytecode']['object'] is not a string")]
    BytecodeObject,
    #[error(transparent)]
    Hex(#[from] hex::FromHexError),
    #[error("Failed to parse HTTP provider URL: {0}")]
    UrlParser(#[from] url::ParseError),
    // CustomError which accepts string
    #[error("Custom error: {0}")]
    CustomError(String),
}
