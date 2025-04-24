// Client abstractions module - contains all client interface traits

pub mod alert;
pub mod event_bus;
pub mod database;
pub mod queue;
pub mod storage;

// Re-export commonly used types
pub use alert::{sns::SNS, AlertClient};
pub use database::{mongodb::MongoDbClient, DatabaseClient};
pub use queue::{sqs::SQS, Message, QueueClient};
pub use storage::{sss::AWSS3, ObjectMetadata, StorageClient};
