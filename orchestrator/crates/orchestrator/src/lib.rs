/// Contains the CLI arguments for the service
pub mod args;
/// Client for the Orchestrator
// pub mod client;

/// Contains the CLI arguments for the service
pub mod constants;
/// Contain the controllers for the service
pub mod controller;

/// Contains the core logic for the service
pub mod core;

/// contains all the error handling / errors that can be returned by the service
pub mod error;
mod macros;
/// contains all the resources that can be used by the service
pub mod resource;
/// Contains all the services that are used by the service
pub mod service;
pub mod setup;
/// Contains utility modules
pub mod util;
/// Contains all the utils that are used by the service
pub mod utils;

// Re-export commonly used ite
pub use error::{OrchestratorError, OrchestratorResult};

// Re-export client abstractions for convenience
pub use core::client::{
    database::DatabaseClient, event_bus::EventBusClient, notification::NotificationClient, queue::QueueClient,
    scheduler::SchedulerClient, storage::StorageClient,
};

/// Initialize the orchestrator library
pub fn init() -> OrchestratorResult<()> {
    // Initialize logging
    utils::logging::init_logging()?;

    // Return success
    Ok(())
}
