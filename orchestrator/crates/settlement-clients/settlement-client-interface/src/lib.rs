use async_trait::async_trait;
use color_eyre::eyre::Result;
use mockall::automock;
use mockall::predicate::*;

pub const SETTLEMENT_SETTINGS_NAME: &str = "settlement_settings";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettlementVerificationStatus {
    Pending,
    Verified,
    Rejected(String),
}

/// Trait for every new Settlement Layer to implement
#[automock]
#[async_trait]
pub trait SettlementClient: Send + Sync {
    /// Should register the proof on the base layer and return an external id
    /// which can be used to track the status.
    async fn register_proof(&self, proof: [u8; 32]) -> Result<String>;

    /// Should be used to update state on core contract when DA is done in calldata
    async fn update_state_calldata(
        &self,
        program_output: Vec<[u8; 32]>,
        onchain_data_hash: [u8; 32],
        onchain_data_size: [u8; 32],
    ) -> Result<String>;

    /// Should be used to update state on contract and publish the blob on ethereum.
    async fn update_state_with_blobs(
        &self,
        program_output: Vec<[u8; 32]>,
        state_diff: Vec<Vec<u8>>,
        nonce: u64,
    ) -> Result<String>;

    /// Should verify the inclusion of a tx in the settlement layer
    async fn verify_tx_inclusion(&self, tx_hash: &str) -> Result<SettlementVerificationStatus>;

    /// Should wait that the pending tx_hash is finalized
    async fn wait_for_tx_finality(&self, tx_hash: &str) -> Result<Option<u64>>;

    /// Should retrieves the last settled block in the settlement layer
    async fn get_last_settled_block(&self) -> Result<u64>;

    /// Should retrieve the latest transaction count to be used as nonce.
    async fn get_nonce(&self) -> Result<u64>;
}
