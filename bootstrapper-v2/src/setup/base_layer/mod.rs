pub mod ethereum;
pub mod starknet;
use anyhow::Result;
use async_trait::async_trait;

use crate::utils::FileError;

#[async_trait]
pub trait BaseLayerSetupTrait {
    /// This function does prerequisite setup for running the base layer setup.
    /// It should be called before the base layer setup.
    async fn init(&mut self) -> Result<(), BaseLayerError>;
    async fn setup(&mut self) -> Result<(), BaseLayerError>;
    async fn post_madara_setup(&mut self, madara_addresses_path: &str) -> Result<(), BaseLayerError>;
}

#[derive(thiserror::Error, Debug)]
pub enum BaseLayerError {
    #[error("Internal base layer error: {0}")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),

    #[error("Ethereum JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Failed to save output: {0}")]
    FailedToSaveOutput(#[from] FileError),

    #[error("Failed to deploy Factory: {0}")]
    FailedToDeployFactory(#[source] anyhow::Error),

    #[error("Failed to read base layer output file: {0}")]
    FailedToReadBaseLayerOutput(#[source] std::io::Error),

    #[error("Failed to read madara output file: {0}")]
    FailedToReadMadaraOutput(#[source] std::io::Error),

    #[error("Key {0} not found in the json string")]
    KeyNotFound(String),
}
