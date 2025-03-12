use async_trait::async_trait;
use bytes::Bytes;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use crate::error::Result;

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
    async fn init(&self) -> Result<()>;

    /// Check if a bucket exists
    async fn bucket_exists(&self, bucket: &str) -> Result<bool>;

    /// Create a bucket
    async fn create_bucket(&self, bucket: &str) -> Result<()>;

    /// Delete a bucket
    async fn delete_bucket(&self, bucket: &str) -> Result<()>;

    /// List objects in a bucket with optional prefix
    async fn list_objects(&self, bucket: &str, prefix: Option<&str>) -> Result<Vec<ObjectMetadata>>;

    /// Upload data to a bucket
    async fn put_object(&self, bucket: &str, key: &str, data: Bytes, content_type: Option<&str>) -> Result<()>;

    /// Upload a file to a bucket
    async fn upload_file(&self, bucket: &str, key: &str, file_path: &Path) -> Result<()>;

    /// Download an object from a bucket
    async fn get_object(&self, bucket: &str, key: &str) -> Result<Bytes>;

    /// Download an object to a file
    async fn download_file(&self, bucket: &str, key: &str, file_path: &Path) -> Result<()>;

    /// Check if an object exists
    async fn object_exists(&self, bucket: &str, key: &str) -> Result<bool>;

    /// Get object metadata
    async fn get_object_metadata(&self, bucket: &str, key: &str) -> Result<ObjectMetadata>;

    /// Delete an object
    async fn delete_object(&self, bucket: &str, key: &str) -> Result<()>;

    /// Generate a presigned URL for an object
    async fn generate_presigned_url(&self, bucket: &str, key: &str, expires_in: Duration) -> Result<String>;
}
