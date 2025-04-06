use async_trait::async_trait;

/// JobHandlerTrait - Trait for job handler
/// This trait defines the methods that a job handler must implement
/// to be able to handle jobs in a job system
#[async_trait]
pub trait JobHandlerTrait: Send + Sync {}
