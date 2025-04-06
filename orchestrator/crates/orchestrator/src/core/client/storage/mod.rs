pub mod error;
pub mod sss;

use async_trait::async_trait;
use bytes::Bytes;
use std::collections::HashMap;

pub use error::StorageError;

/// Object metadata
#[derive(Debug, Clone)]
pub struct ObjectMetadata {
    /// Object key
    pub key: String,

    /// Object size in bytes
    pub size: u64,

    /// Last modified timestamp
    pub last_modified: Option<chrono::DateTime<chrono::Utc>>,

    /// ETag
    pub etag: Option<String>,

    /// Content type
    pub content_type: Option<String>,

    /// User-defined metadata
    pub metadata: HashMap<String, String>,
}

/// Trait defining object storage operations
#[async_trait]
pub trait StorageClient: Send + Sync {
    /// Initialize the storage client
    async fn get_data(&self, key: &str) -> Result<Bytes, StorageError>;

    /// Check if a bucket exists
    async fn put_data(&self, data: Bytes, key: &str) -> Result<(), StorageError>;
    /// Delete a bucket
    async fn delete_data(&self, key: &str) -> Result<(), StorageError>;
}
