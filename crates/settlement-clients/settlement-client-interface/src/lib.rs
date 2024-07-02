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
        onchain_data_size: usize,
    ) -> Result<String>;

    /// Should be used to update state on core contract when DA is in blobs/alt DA
    async fn update_state_blobs(&self, program_output: Vec<[u8; 32]>, kzg_proof: [u8; 48]) -> Result<String>;

    /// Should verify the inclusion of a tx in the settlement layer
    async fn verify_tx_inclusion(&self, tx_hash: &str) -> Result<SettlementVerificationStatus>;

    /// Should wait that the pending tx_hash is finalized
    async fn wait_for_tx_finality(&self, tx_hash: &str) -> Result<()>;

    /// Should retrieves the last settled block in the settlement layer
    async fn get_last_settled_block(&self) -> Result<u64>;
}

/// Trait for every new SettlementConfig to implement
pub trait SettlementConfig {
    /// Should create a new instance of the SettlementConfig from the environment variables
    fn new_from_env() -> Self;
}
