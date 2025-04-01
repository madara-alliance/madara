// Client abstractions module - contains all client interface traits

pub mod database;
pub mod event_bus;
pub mod notification;
pub mod queue;
pub mod scheduler;
pub mod storage;
pub mod alert;

// Re-export commonly used types
pub use database::DatabaseClient;
pub use event_bus::{Event, EventBusClient, RuleTarget};
pub use notification::{Notification, NotificationClient};
pub use queue::{Message, QueueClient};
pub use scheduler::{ScheduleTarget, ScheduleType, SchedulerClient};
pub use storage::{ObjectMetadata, StorageClient};
