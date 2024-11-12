pub mod aws_s3;
pub mod types;

use async_trait::async_trait;
use bytes::Bytes;
use color_eyre::Result;
use mockall::automock;
use utils::settings::Settings;

/// Data Storage Trait
///
/// DataStorage trait contains the functions used to store and get the data from
/// the cloud provider storage.
/// The proposed storage format is :
///     ----<block_number>
///         ----<cairo_pie.json> (stored during the SNOS job)
///         ----<snos_output.json> (stored during the SNOS job)
///         ----<blob_data.txt> (stored during the DA job)
#[automock]
#[async_trait]
pub trait DataStorage: Send + Sync {
    async fn get_data(&self, key: &str) -> Result<Bytes>;
    async fn put_data(&self, data: Bytes, key: &str) -> Result<()>;
    async fn create_bucket(&self, bucket_name: &str) -> Result<()>;
    async fn setup(&self, settings_provider: Box<dyn Settings>) -> Result<()> {
        let bucket_name = settings_provider.get_settings_or_panic("STORAGE_BUCKET_NAME");
        self.create_bucket(&bucket_name).await
    }
}

/// **DataStorageConfig** : Trait method to represent the config struct needed for
/// initialisation of data storage client
pub trait DataStorageConfig {
    /// Get a config file from environment vars in system or
    fn new_with_settings(settings: &impl Settings) -> Self;
}
