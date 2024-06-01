use async_trait::async_trait;
use color_eyre::Result;
use mockall::{automock, predicate::*};
use starknet::core::types::FieldElement;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SettlementVerificationStatus {
    #[allow(dead_code)]
    Pending,
    #[allow(dead_code)]
    Verified,
    #[allow(dead_code)]
    Rejected,
}

/// Trait for every new DaClient to implement
#[automock]
#[async_trait]
pub trait SettlementClient: Send + Sync {
    /// Should register the proof on the base layer and return an external id
    /// which can be used to track the status.
    async fn register_proof(&self, proof: Vec<FieldElement>) -> Result<String>;

    /// Should be used to update state on core contract when DA is done in calldata
    async fn update_state_calldata(
        &self,
        program_output: Vec<FieldElement>,
        onchain_data_hash: FieldElement,
        onchain_data_size: FieldElement,
    ) -> Result<String>;

    /// Should be used to update state on core contract when DA is in blobs/alt DA
    async fn update_state_blobs(&self, program_output: Vec<FieldElement>, kzg_proof: Vec<u8>) -> Result<String>;

    /// Should verify the inclusion of the state diff in the DA layer and return the status
    async fn verify_inclusion(&self, external_id: &str) -> Result<SettlementVerificationStatus>;
}

/// Trait for every new DaConfig to implement
pub trait SettlementConfig {
    /// Should create a new instance of the DaConfig from the environment variables
    fn new_from_env() -> Self;
}
