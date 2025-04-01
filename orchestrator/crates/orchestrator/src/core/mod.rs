// Core module - contains all the core abstractions

pub mod client;
pub mod cloud;
pub mod config;

// Re-export commonly used types from client
pub use client::database::DatabaseClient;
pub use client::event_bus::{Event, EventBusClient, RuleTarget};
pub use client::notification::{Notification, NotificationClient};
pub use client::queue::{Message, QueueClient};
pub use client::scheduler::{ScheduleTarget, ScheduleType, SchedulerClient};
pub use client::storage::{ObjectMetadata, StorageClient};

