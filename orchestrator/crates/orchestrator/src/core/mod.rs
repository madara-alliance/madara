// Core module - contains all the core abstractions

pub mod client;
pub mod madara;
pub mod cloud;

// Re-export commonly used types from client
pub use client::database::DatabaseClient;
pub use client::event_bus::{Event, EventBusClient, RuleTarget};
pub use client::notification::{Notification, NotificationClient};
pub use client::queue::{Message, QueueClient};
pub use client::scheduler::{ScheduleTarget, ScheduleType, SchedulerClient};
pub use client::storage::{ObjectMetadata, StorageClient};

// Re-export Madara-specific types
pub use madara::cron::{CronClient, TargetConfig, TriggerType};
pub use madara::da::{DaClient, DaVerificationStatus, PublicationResult};
pub use madara::job_queue::{Delivery, JobQueueClient, JobType, QueueConfig, QueueType};
pub use madara::settlement::{SettlementClient, SettlementVerificationStatus, StateUpdate};
