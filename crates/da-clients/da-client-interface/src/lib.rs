use async_trait::async_trait;
use color_eyre::Result;
use mockall::automock;
use mockall::predicate::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaVerificationStatus {
    #[allow(dead_code)]
    Pending,
    #[allow(dead_code)]
    Verified,
    #[allow(dead_code)]
    Rejected(String),
}

/// Trait for every new DaClient to implement
#[automock]
#[async_trait]
pub trait DaClient: Send + Sync {
    /// Should publish the state diff to the DA layer and return an external id
    /// which can be used to track the status of the DA transaction.
    async fn publish_state_diff(&self, state_diff: Vec<Vec<u8>>, to: &[u8; 32]) -> Result<String>;
    /// Should verify the inclusion of the state diff in the DA layer and return the status
    async fn verify_inclusion(&self, external_id: &str) -> Result<DaVerificationStatus>;
    /// Should return the max blobs per txn
    async fn max_blob_per_txn(&self) -> u64;
    /// Should return the max bytes per blob
    async fn max_bytes_per_blob(&self) -> u64;
}

/// Trait for every new DaConfig to implement
pub trait DaConfig {
    /// Should create a new instance of the DaConfig from the environment variables
    fn new_from_env() -> Self;
}
