use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Data availability verification status
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DaVerificationStatus {
    /// Data is pending inclusion
    Pending,

    /// Data is verified as included
    Verified,

    /// Data inclusion was rejected with reason
    Rejected(String),
}

/// State diff publication options
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicationOptions {
    /// Chunk state diff into multiple parts
    pub chunked: bool,

    /// Maximum size per chunk (in bytes)
    pub max_chunk_size: Option<u64>,

    /// Gas fee (if applicable)
    pub gas_fee: Option<u64>,

    /// Priority fee (if applicable)
    pub priority_fee: Option<u64>,
}

/// State diff publication result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicationResult {
    /// External ID or transaction hash
    pub external_id: String,

    /// Number of chunks used
    pub chunk_count: u32,

    /// Total data size published (in bytes)
    pub total_size: u64,

    /// Fee paid (if applicable)
    pub fee_paid: Option<u64>,
}

/// Trait defining data availability operations
#[async_trait]
pub trait DaClient: Send + Sync {
    /// Initialize the DA client
    async fn init(&self) -> Result<()>;

    /// Get the name of the DA provider
    fn provider_name(&self) -> &str;

    /// Get the DA layer constraints
    async fn get_constraints(&self) -> Result<(u64, u64)>;

    /// Publish state diff to the DA layer
    async fn publish_state_diff(
        &self,
        state_diff: Vec<Vec<u8>>,
        recipient: Option<&[u8; 32]>,
        options: Option<PublicationOptions>,
    ) -> Result<PublicationResult>;

    /// Verify the inclusion of state diff in the DA layer
    async fn verify_inclusion(&self, external_id: &str) -> Result<DaVerificationStatus>;

    /// Get the transaction details for a published state diff
    async fn get_transaction_details(&self, external_id: &str) -> Result<Option<serde_json::Value>>;

    /// Check if the DA client is connected to the network
    async fn is_connected(&self) -> Result<bool>;

    /// Validate state diff size against DA layer constraints
    async fn validate_state_diff(&self, state_diff: &[Vec<u8>]) -> Result<bool> {
        let (max_blob_count, max_blob_size) = self.get_constraints().await?;

        // Check blob count
        if state_diff.len() as u64 > max_blob_count {
            return Ok(false);
        }

        // Check individual blob sizes
        for blob in state_diff {
            if blob.len() as u64 > max_blob_size {
                return Ok(false);
            }
        }

        Ok(true)
    }
}
