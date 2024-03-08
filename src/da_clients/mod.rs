use crate::jobs::types::JobVerificationStatus;
use axum::async_trait;
use color_eyre::Result;
use starknet::core::types::FieldElement;

pub mod ethereum;

#[async_trait]
pub trait DaClient: Send + Sync {
    async fn publish_state_diff(&self, state_diff: Vec<FieldElement>) -> Result<String>;
    async fn verify_inclusion(&self, external_id: &String) -> Result<JobVerificationStatus>;
}

pub trait DaConfig {
    fn new_from_env() -> Self;
}
