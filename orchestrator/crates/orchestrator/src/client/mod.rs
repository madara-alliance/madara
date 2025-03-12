// Client module - contains all client implementations

pub mod db;
pub mod storage;
// Additional client modules will be added here:
// pub mod queue;
// pub mod notification;
// pub mod event_bus;
// pub mod scheduler;

// Re-export commonly used clients
pub use db::MongoDbClient;
pub use storage::S3StorageClient;
