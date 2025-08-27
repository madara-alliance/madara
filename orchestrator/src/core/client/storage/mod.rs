pub mod error;
pub mod s3;

use async_trait::async_trait;
use bytes::Bytes;
pub use error::StorageError;

/// Trait defining object storage operations
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait StorageClient: Send + Sync {
    /// Initialize the storage client
    async fn get_data(&self, key: &str) -> Result<Bytes, StorageError>;

    /// Check if a bucket exists
    async fn put_data(&self, data: Bytes, key: &str) -> Result<(), StorageError>;
    /// Delete a bucket
    async fn delete_data(&self, key: &str) -> Result<(), StorageError>;

    /// List files in a directory
    async fn list_files_in_dir(&self, dir_path: &str) -> Result<Vec<String>, StorageError>;
}
