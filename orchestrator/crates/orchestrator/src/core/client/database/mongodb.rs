use async_trait::async_trait;
use futures::TryStreamExt;
use mongodb::{
    bson::{Document},
    Client, Database,
};
use serde_json::Value;
use std::sync::Arc;

use crate::{
    core::client::database::{DatabaseClient, DatabaseClientExt},
    error::OrchestratorError,
    error::OrchestratorResult,
    params::database::MongoConfig,
};

/// MongoDB client implementation
pub struct MongoDbClient {
    client: Client,
    database: Arc<Database>,
}

impl MongoDbClient {
    /// Get a reference to the database
    fn get_database(&self) -> OrchestratorResult<Arc<Database>> {
        Ok(self.database.clone())
    }
    pub async fn setup(config: &MongoConfig) -> OrchestratorResult<Self> {
        let client = Client::with_uri_str(&config.connection_url).await?;
        let database = Arc::new(client.database(&config.database_name));
        Ok(Self { client, database })
    }
}

#[async_trait]
impl DatabaseClient for MongoDbClient {
    async fn switch_database(&mut self, database_name: &str) -> OrchestratorResult<()> {
        self.database = Arc::new(self.client.database(database_name));
        Ok(())
    }

    async fn disconnect(&self) -> OrchestratorResult<()> {
        // MongoDB client automatically disconnects when dropped
        Ok(())
    }

    async fn insert_json(&self, table: &str, document: &str) -> OrchestratorResult<String> {
        let doc: Document = mongodb::bson::from_slice(document.as_bytes())
            .map_err(|e| OrchestratorError::DatabaseError(e.to_string()))?;
        let result = self.database.collection::<Document>(table).insert_one(doc, None).await?;
        Ok(result.inserted_id.to_string())
    }

    async fn find_document_json(&self, collection: &str, filter: &str) -> OrchestratorResult<Option<String>> {
        let filter_doc: Document = mongodb::bson::from_slice(filter.as_bytes())
            .map_err(|e| OrchestratorError::DatabaseError(e.to_string()))?;
        let result = self.database.collection::<Document>(collection).find_one(filter_doc, None).await?;

        match result {
            Some(doc) => {
                let value: Value =
                    mongodb::bson::from_document(doc).map_err(|e| OrchestratorError::DatabaseError(e.to_string()))?;
                Ok(Some(value.to_string()))
            }
            None => Ok(None),
        }
    }

    async fn find_documents_json(&self, collection: &str, filter: &str) -> OrchestratorResult<Vec<String>> {
        let filter_doc: Document = mongodb::bson::from_slice(filter.as_bytes())
            .map_err(|e| OrchestratorError::DatabaseError(e.to_string()))?;
        let cursor = self.database.collection::<Document>(collection).find(filter_doc, None).await?;

        let mut documents = Vec::new();
        let mut stream = cursor;
        while let Some(doc) = stream.try_next().await? {
            let value: Value =
                mongodb::bson::from_document(doc).map_err(|e| OrchestratorError::DatabaseError(e.to_string()))?;
            documents.push(value.to_string());
        }
        Ok(documents)
    }

    async fn update_document_json(&self, collection: &str, filter: &str, update: &str) -> OrchestratorResult<bool> {
        let filter_doc: Document = mongodb::bson::from_slice(filter.as_bytes())
            .map_err(|e| OrchestratorError::DatabaseError(e.to_string()))?;
        let update_doc: Document = mongodb::bson::from_slice(update.as_bytes())
            .map_err(|e| OrchestratorError::DatabaseError(e.to_string()))?;
        let result = self.database.collection::<Document>(collection).update_one(filter_doc, update_doc, None).await?;
        Ok(result.modified_count > 0)
    }

    async fn delete_document_json(&self, collection: &str, filter: &str) -> OrchestratorResult<bool> {
        let filter_doc: Document = mongodb::bson::from_slice(filter.as_bytes())
            .map_err(|e| OrchestratorError::DatabaseError(e.to_string()))?;
        let result = self.database.collection::<Document>(collection).delete_one(filter_doc, None).await?;
        Ok(result.deleted_count > 0)
    }

    async fn count_documents_json(&self, collection: &str, filter: &str) -> OrchestratorResult<u64> {
        let filter_doc: Document = mongodb::bson::from_slice(filter.as_bytes())
            .map_err(|e| OrchestratorError::DatabaseError(e.to_string()))?;
        let count = self.database.collection::<Document>(collection).count_documents(filter_doc, None).await?;
        Ok(count)
    }
}

impl DatabaseClientExt for MongoDbClient {}
