use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Settlement verification status
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SettlementVerificationStatus {
    /// Transaction is pending
    Pending,

    /// Transaction is verified
    Verified,

    /// Transaction is rejected with reason
    Rejected(String),
}

/// Program output data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgramOutput {
    /// Output data (array of 32-byte values)
    pub data: Vec<[u8; 32]>,
}

/// State update data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateUpdate {
    /// Program output
    pub program_output: ProgramOutput,

    /// State diff data (optional, for blob-based updates)
    pub state_diff: Option<Vec<Vec<u8>>>,

    /// On-chain data hash (for calldata-based updates)
    pub onchain_data_hash: Option<[u8; 32]>,

    /// On-chain data size (for calldata-based updates)
    pub onchain_data_size: Option<[u8; 32]>,
}

/// Trait defining settlement layer operations
#[async_trait]
pub trait SettlementClient: Send + Sync {
    /// Initialize the settlement client
    async fn init(&self) -> Result<()>;

    /// Update state on the settlement layer
    async fn update_state(&self, state_update: StateUpdate, nonce: Option<u64>) -> Result<String>;

    /// Verify the inclusion of a transaction in the settlement layer
    async fn verify_transaction(&self, tx_hash: &str) -> Result<SettlementVerificationStatus>;

    /// Wait for a transaction to be finalized
    ///
    /// Returns the block number where the transaction was finalized, if available
    async fn wait_for_finality(&self, tx_hash: &str) -> Result<Option<u64>>;

    /// Get the last settled block number
    async fn get_last_settled_block(&self) -> Result<u64>;

    /// Get the current nonce
    async fn get_nonce(&self) -> Result<u64>;

    /// Get the chain ID
    async fn get_chain_id(&self) -> Result<u64>;

    /// Check if the client is connected to the network
    async fn is_connected(&self) -> Result<bool>;
}
