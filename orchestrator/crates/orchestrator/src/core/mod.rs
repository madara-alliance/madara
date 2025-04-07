// Core module - contains all the core abstractions

pub mod client;
pub mod cloud;
pub mod config;
pub mod error;
pub mod traits;

// Re-export commonly used types from client
pub use client::database::DatabaseClient;
pub use client::queue::{Message, QueueClient};
pub use client::storage::{ObjectMetadata, StorageClient};
