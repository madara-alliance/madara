pub mod sss;

use async_trait::async_trait;
use bytes::Bytes;
use std::collections::HashMap;

use crate::OrchestratorResult;

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
    async fn get_data(&self, key: String) -> OrchestratorResult<Bytes>;

    /// Check if a bucket exists
    async fn put_data(&self, data: Bytes, key: String) -> OrchestratorResult<()>;

    /// Create a bucket
    async fn create_bucket(&self, bucket: String) -> OrchestratorResult<()>;

    /// Delete a bucket
    async fn delete_bucket(&self, bucket: String) -> OrchestratorResult<()>;
    /// Delete a bucket
    async fn delete_data(&self, key: String) -> OrchestratorResult<()>;
}
