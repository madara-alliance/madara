// Client abstractions module - contains all client interface traits

pub mod alert;
pub mod database;
pub mod event_bus;
pub mod lock;
pub mod queue;
pub mod storage;

// Re-export commonly used types
pub use alert::{sns::SNS, AlertClient};
pub use database::{mongodb::MongoDbClient, DatabaseClient};
pub use queue::{sqs::SQS, QueueClient};
pub use storage::{s3::AWSS3, StorageClient};
