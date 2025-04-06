// Client abstractions module - contains all client interface traits

pub mod database;
pub mod queue;
pub mod storage;
pub mod alert;

// Re-export commonly used types
pub use database::{DatabaseClient, mongodb::MongoDbClient};
pub use queue::{Message, QueueClient, sqs::SQS};
pub use storage::{ObjectMetadata, StorageClient, sss::AWSS3};
pub use alert::{AlertClient, sns::SNS};

