// Client module - contains all client implementations

pub mod db;
pub mod storage;

// Re-export commonly used clients
pub use db::MongoDbClient;
pub use storage::S3StorageClient;
