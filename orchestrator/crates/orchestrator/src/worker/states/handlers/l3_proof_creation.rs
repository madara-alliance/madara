use super::super::error::StateHandlerError;
use super::super::StateHandler;
use crate::core::config::Config;
use crate::worker::states::types::product::StateProduct;
use async_trait::async_trait;
use std::sync::Arc;
use uuid::Uuid;

pub struct L3ProofCreationHandler(Arc<Config>);

#[async_trait]
impl StateHandler for L3ProofCreationHandler {

    async fn new(config: Arc<Config>) -> Self { Self(config) }

    async fn process(&self, job_id: Uuid) -> Result<StateProduct, StateHandlerError> {
        todo!()
    }

    async fn validate_input(&self, job_id: Uuid) -> Result<bool, StateHandlerError> {
        todo!()
    }

    async fn persist_state(&self, job_id: Uuid, state: &StateProduct) -> Result<(), StateHandlerError> {
        todo!()
    }
}