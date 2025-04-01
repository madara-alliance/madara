use async_trait::async_trait;
use mongodb::{
    bson::{doc, Document},
    options::ClientOptions,
    Client as MongoClient,
};
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    OrchestratorResult, OrchestratorError,
    core::client::database::DatabaseClient,
};

/// MongoDB client implementation
pub struct MongoDbClient {
    client: Option<MongoClient>,
    connection_string: String,
    database: String,
}

impl MongoDbClient {
    /// Create a new MongoDB client
    pub fn new(connection_string: &str, database: &str) -> Self {
        Self { client: None, connection_string: connection_string.to_string(), database: database.to_string() }
    }

    /// Get a reference to the MongoDB client
    fn get_client(&self) -> OrchestratorResult<&MongoClient> {
        self.client.as_ref().ok_or(OrchestratorError::DatabaseError("MongoDB client not initialized".to_string()))
    }

    /// Get a reference to the database
    fn get_database(&self) -> OrchestratorResult<mongodb::Database> {
        let client = self.get_client()?;
        Ok(client.database(&self.database))
    }
}

#[async_trait]
impl DatabaseClient for MongoDbClient {
    async fn connect(&mut self) -> OrchestratorResult<()> {
        let client_options = ClientOptions::parse(&self.connection_string).await?;
        let client = MongoClient::with_options(client_options)?;
        client
            .database("admin")
            .run_command(doc! { "ping": 1 }, None)
            .await?;
        self.client = Some(client);
        Ok(())
    }

    /// disconnect - will disconnect the mongodb client
    async fn disconnect(&mut self) -> OrchestratorResult<()> {
        self.client = None;
        Ok(())
    }

    async fn insert<T>(&self, table: &str, document: &T) -> OrchestratorResult<Option<String>>
    where
        T: Serialize + Send + Sync,
    {
        let db = self.get_database()?;
        let coll = db.collection::<Document>(table);
        let doc = mongodb::bson::to_document(document)?;

        let result = coll
            .insert_one(doc, None)
            .await?;

        let id = result
            .inserted_id
            .as_object_id().map(|id| id.to_hex());
        Ok(id)
    }

    async fn find_document<T, F>(&self, collection: &str, filter: F) -> OrchestratorResult<Option<T>>
    where
        T: DeserializeOwned + Send + Sync,
        F: Serialize + Send + Sync,
    {
        let db = self.get_database()?;
        let coll = db.collection::<T>(collection);
        let filter_doc = mongodb::bson::to_document(&filter)?;
        let result = coll
            .find_one(filter_doc, None)
            .await?;
        Ok(result)
    }

    async fn find_documents<T, F>(&self, collection: &str, filter: F) -> Result<Vec<T>>
    where
        T: DeserializeOwned + Send + Sync,
        F: Serialize + Send + Sync,
    {
        let db = self.get_database()?;
        let coll = db.collection::<T>(collection);

        let filter_doc = mongodb::bson::to_document(&filter)
            .map_err(|e| Error::DatabaseError(format!("Failed to serialize filter: {}", e)))?;

        let cursor = coll
            .find(filter_doc, None)
            .await
            .map_err(|e| Error::DatabaseError(format!("Failed to find documents: {}", e)))?;

        let results = cursor
            .try_collect()
            .await
            .map_err(|e| Error::DatabaseError(format!("Failed to collect documents: {}", e)))?;

        Ok(results)
    }

    async fn update_document<F, U>(&self, collection: &str, filter: F, update: U) -> Result<bool>
    where
        F: Serialize + Send + Sync,
        U: Serialize + Send + Sync,
    {
        let db = self.get_database()?;
        let coll = db.collection::<Document>(collection);

        let filter_doc = mongodb::bson::to_document(&filter)
            .map_err(|e| Error::DatabaseError(format!("Failed to serialize filter: {}", e)))?;

        let update_doc = mongodb::bson::to_document(&update)
            .map_err(|e| Error::DatabaseError(format!("Failed to serialize update: {}", e)))?;

        let result = coll
            .update_one(filter_doc, update_doc, None)
            .await
            .map_err(|e| Error::DatabaseError(format!("Failed to update document: {}", e)))?;

        Ok(result.modified_count > 0)
    }

    async fn delete_document<F>(&self, collection: &str, filter: F) -> Result<bool>
    where
        F: Serialize + Send + Sync,
    {
        let db = self.get_database()?;
        let coll = db.collection::<Document>(collection);

        let filter_doc = mongodb::bson::to_document(&filter)
            .map_err(|e| Error::DatabaseError(format!("Failed to serialize filter: {}", e)))?;

        let result = coll
            .delete_one(filter_doc, None)
            .await
            .map_err(|e| Error::DatabaseError(format!("Failed to delete document: {}", e)))?;

        Ok(result.deleted_count > 0)
    }

    async fn count_documents<F>(&self, collection: &str, filter: F) -> Result<u64>
    where
        F: Serialize + Send + Sync,
    {
        let db = self.get_database()?;
        let coll = db.collection::<Document>(collection);

        let filter_doc = mongodb::bson::to_document(&filter)
            .map_err(|e| Error::DatabaseError(format!("Failed to serialize filter: {}", e)))?;

        let count = coll
            .count_documents(filter_doc, None)
            .await
            .map_err(|e| Error::DatabaseError(format!("Failed to count documents: {}", e)))?;

        Ok(count)
    }
}
