pub mod error;
pub mod types;
pub mod handlers;
mod pipeline;
mod orchestrator;

use crate::core::config::Config;
use crate::worker::states::types::product::StateProduct;
use async_trait::async_trait;
use error::StateHandlerError;
use std::sync::Arc;
use uuid::Uuid;

/// StateHandler is a trait that defines the interface for all state handlers.
#[async_trait]
pub(crate) trait StateHandler: Send + Sync {
    async fn new(config: Arc<Config>) -> Self
    where
        Self: Sized;
    async fn process(&self, job_id: Uuid) -> Result<StateProduct, StateHandlerError>;

    /// validate_input is used to validate the input of the state handler.
    /// If the input is valid to process
    async fn validate_input(&self, job_id: Uuid) -> Result<bool, StateHandlerError>;

    /// persist_state is used to persist the state of the job. after the state is
    /// processed, the state is persisted to the database.
    ///
    /// # Arguments
    /// * `job_id` - The id of the job to persist the state for.
    /// * `state` - The state to persist.
    ///
    /// # Returns
    /// * `Result<(), StateHandlerError>` - Ok, if the state was persisted successfully, Err otherwise.
    async fn persist_state(
        &self,
        job_id: Uuid,
        state: &StateProduct,
    ) -> Result<(), StateHandlerError>;
}