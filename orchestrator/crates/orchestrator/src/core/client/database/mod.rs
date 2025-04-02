pub mod mongodb;

use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};

use crate::error::OrchestratorResult;

/// Trait defining database operations
#[async_trait]
pub trait DatabaseClient: Send + Sync {
    // type ConnectArgs: Clone;
    // /// Setup the database client
    // async fn setup(args: Self::ConnectArgs) -> OrchestratorResult<Self>;
    /// switch to a different database
    async fn switch_database(&mut self, database_name: &str) -> OrchestratorResult<()>;

    /// Disconnect from the database
    async fn disconnect(&self) -> OrchestratorResult<()>;

    // Instead of generic methods, we'll use concrete types with serialization
    async fn insert_json(&self, table: &str, document: &str) -> OrchestratorResult<String>;
    async fn find_document_json(&self, collection: &str, filter: &str) -> OrchestratorResult<Option<String>>;
    async fn find_documents_json(&self, collection: &str, filter: &str) -> OrchestratorResult<Vec<String>>;
    async fn update_document_json(&self, collection: &str, filter: &str, update: &str) -> OrchestratorResult<bool>;
    async fn delete_document_json(&self, collection: &str, filter: &str) -> OrchestratorResult<bool>;
    async fn count_documents_json(&self, collection: &str, filter: &str) -> OrchestratorResult<u64>;
}

// Helper trait for type-safe operations
#[async_trait]
pub trait DatabaseClientExt: DatabaseClient {
    async fn insert<T>(&self, table: &str, document: &T) -> OrchestratorResult<String>
    where
        T: Serialize + Send + Sync,
    {
        let json = serde_json::to_string(document)?;
        self.insert_json(table, &json).await
    }

    async fn find_document<T, F>(&self, collection: &str, filter: &F) -> OrchestratorResult<Option<T>>
    where
        T: DeserializeOwned + Send + Sync,
        F: Serialize + Send + Sync,
    {
        let filter_json = serde_json::to_string(filter)?;
        let result = self.find_document_json(collection, &filter_json).await?;
        match result {
            Some(json) => Ok(Some(serde_json::from_str(&json)?)),
            None => Ok(None),
        }
    }

    async fn find_documents<T, F>(&self, collection: &str, filter: &F) -> OrchestratorResult<Vec<T>>
    where
        T: DeserializeOwned + Send + Sync,
        F: Serialize + Send + Sync,
    {
        let filter_json = serde_json::to_string(filter)?;
        let results = self.find_documents_json(collection, &filter_json).await?;
        results.into_iter().map(|json| Ok(serde_json::from_str(&json)?)).collect()
    }

    async fn update_document<F, U>(&self, collection: &str, filter: &F, update: &U) -> OrchestratorResult<bool>
    where
        F: Serialize + Send + Sync,
        U: Serialize + Send + Sync,
    {
        let filter_json = serde_json::to_string(filter)?;
        let update_json = serde_json::to_string(update)?;
        self.update_document_json(collection, &filter_json, &update_json).await
    }

    async fn delete_document<F>(&self, collection: &str, filter: &F) -> OrchestratorResult<bool>
    where
        F: Serialize + Send + Sync,
    {
        let filter_json = serde_json::to_string(filter)?;
        self.delete_document_json(collection, &filter_json).await
    }

    async fn count_documents<F>(&self, collection: &str, filter: &F) -> OrchestratorResult<u64>
    where
        F: Serialize + Send + Sync,
    {
        let filter_json = serde_json::to_string(filter)?;
        self.count_documents_json(collection, &filter_json).await
    }
}
