use async_trait::async_trait;

pub mod error;
pub mod event_bridge;

/// EventBusClient: Trait defining Event Bus Operation
///
/// Note: This trait is used to interact with the Event Bus system
/// there is not Abstract code for this now since this is used only in setup not anywhere else
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait EventBusClient: Send + Sync {}
