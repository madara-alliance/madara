pub mod mongodb;

use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};

use crate::error::Result;
use crate::OrchestratorResult;

/// Trait defining database operations
#[async_trait]
pub trait DatabaseClient: Send + Sync {
    // type ConnectArgs: Clone;
    // /// Setup the database client
    // async fn setup(args: Self::ConnectArgs) -> OrchestratorResult<Self>;
    /// switch to a different database
    async fn switch_database(&self, database_name: &str) -> OrchestratorResult<()>;

    /// Disconnect from the database
    async fn disconnect(&self) -> Result<()>;

    /// Insert a document into a table
    async fn insert<T>(&self, table: &str, document: &T) -> OrchestratorResult<String>
    where
        T: Serialize + Send + Sync;

    /// Find a document in a collection by filter
    async fn find_document<T, F>(&self, collection: &str, filter: F) -> OrchestratorResult<Option<T>>
    where
        T: DeserializeOwned + Send + Sync,
        F: Serialize + Send + Sync;

    /// Find multiple documents in a collection by filter
    async fn find_documents<T, F>(&self, collection: &str, filter: F) -> OrchestratorResult<Vec<T>>
    where
        T: DeserializeOwned + Send + Sync,
        F: Serialize + Send + Sync;

    /// Update a document in a collection
    async fn update_document<F, U>(&self, collection: &str, filter: F, update: U) -> OrchestratorResult<bool>
    where
        F: Serialize + Send + Sync,
        U: Serialize + Send + Sync;

    /// Delete a document from a collection
    async fn delete_document<F>(&self, collection: &str, filter: F) -> OrchestratorResult<bool>
    where
        F: Serialize + Send + Sync;

    /// Count documents in a collection matching a filter
    async fn count_documents<F>(&self, collection: &str, filter: F) -> OrchestratorResult<u64>
    where
        F: Serialize + Send + Sync;
}
