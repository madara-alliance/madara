#[derive(Debug, thiserror::Error)]
pub enum LockError {
    #[error("MongoDB operation failed: {0}")]
    MongoDB(#[from] mongodb::error::Error),

    #[error("Serialization failed: {0}")]
    Serialization(#[from] mongodb::bson::ser::Error),

    #[error("Deserialization failed: {0}")]
    Deserialization(#[from] mongodb::bson::de::Error),

    #[error("JSON serialization failed: {0}")]
    JsonSerialization(#[from] serde_json::Error),

    #[error("Key not found: {0}")]
    KeyNotFound(String),

    #[error("Invalid key format: {0}")]
    InvalidKey(String),

    #[error("Invalid value type: expected {expected}, got {actual}")]
    InvalidValueType { expected: String, actual: String },

    #[error("TTL value out of range: {0} (must be between 1 and 2147483647 seconds)")]
    InvalidTtl(i64),

    #[error("Pattern is invalid: {0}")]
    InvalidPattern(String),

    #[error("Value too large: {size} bytes (max allowed: {max} bytes)")]
    ValueTooLarge { size: usize, max: usize },

    #[error("Operation failed: {0}")]
    OperationFailed(String),

    #[error("Atomic operation failed: {0}")]
    AtomicOperationFailed(String),

    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Database initialization failed: {0}")]
    InitializationFailed(String),

    #[error("Lock operation failed: {0}")]
    LockOperationFailed(String),

    #[error("Lock already held by different owner: {current_owner}")]
    LockAlreadyHeld { current_owner: String },
}
