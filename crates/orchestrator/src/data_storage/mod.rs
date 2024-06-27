mod aws_s3;
mod types;

use async_trait::async_trait;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Error;
use bytes::Bytes;
use mockall::automock;

/// DataStorage trait contains the functions used to store and get the data from
/// the cloud provider storage.
/// The proposed storage format is :
/// ----s3
///     ----<block_number>
///         ----<snos_output.json>
///         ----<kzg.txt>
#[automock]
#[async_trait]
pub trait DataStorage: Send + Sync {
    async fn get_data(&self, key: &str) -> Result<Bytes, Error>;
    async fn put_data(&self, data: ByteStream, key: &str) -> Result<(), Error>;
}

/// **DataStorageConfig** : Trait method to represent the config struct needed for
/// initialisation of data storage client
pub trait DataStorageConfig {
    /// Get a config file from environment vars in system or
    /// dotenv file.
    fn new_from_env() -> Self;
}
