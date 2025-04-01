use std::sync::Arc;
use async_trait::async_trait;
use futures::TryStreamExt;
use mongodb::{bson::{doc, Document}, options::ClientOptions, Client as MongoClient, Database};
use serde::{de::DeserializeOwned, Serialize};

use crate::{OrchestratorResult, OrchestratorError, DatabaseClient};
use crate::params::database::MongoConfig;

/// MongoDB client implementation
pub struct MongoDbClient {
    client: Option<Arc<MongoClient>>,
    connection_string: String,
    database: Arc<Database>,
}

impl MongoDbClient {
    /// Get a reference to the MongoDB client
    fn get_client(&self) -> OrchestratorResult<&MongoClient> {
        let cli = self.client.as_ref().ok_or(OrchestratorError::DatabaseError("MongoDB client not initialized".to_string()))?;
        Ok(cli.as_ref())
    }

    /// Get a reference to the database
    fn get_database(&self) -> OrchestratorResult<Arc<Database>> {
        Ok(self.database.clone())
    }
    pub async fn setup(args: MongoConfig) -> OrchestratorResult<Self> {
        let client_options = ClientOptions::parse(&args.connection_url).await?;
        let client = MongoClient::with_options(client_options)?;
        let database = Arc::new(client.database(&args.database_name));
        database
            .run_command(doc! { "ping": 1 }, None)
            .await?;
        Ok(Self { client: Some(Arc::new(client)), connection_string, database })
    }
}

#[async_trait]
impl DatabaseClient for MongoDbClient {
    async fn switch_database(&mut self, database_name: &str) -> OrchestratorResult<()> {
        let client = self.get_client()?;
        self.database = Arc::new(client.database(database_name));
        Ok(())
    }

    async fn disconnect(&self) -> OrchestratorResult<()> {
        // MongoDB client automatically disconnects when dropped
        Ok(())
    }

    async fn insert<T>(&self, table: &str, document: &T) -> OrchestratorResult<String>
    where
        T: Serialize + Send + Sync,
    {
        let db = self.get_database()?;
        let coll = db.collection::<Document>(table);

        let doc = mongodb::bson::to_document(document)?;

        let result = coll.insert_one(doc, None).await?;

        let id = result
            .inserted_id.as_object_id().ok_or_else(|| OrchestratorError::DatabaseError("Failed to get inserted document ID".to_string()))?;

        Ok(id.to_hex())
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

    async fn find_documents<T, F>(&self, collection: &str, filter: F) -> OrchestratorResult<Vec<T>>
    where
        T: DeserializeOwned + Send + Sync,
        F: Serialize + Send + Sync,
    {
        let db = self.get_database()?;
        let coll = db.collection::<T>(collection);

        let filter_doc = mongodb::bson::to_document(&filter)?;

        let cursor = coll
            .find(filter_doc, None)
            .await?;

        let results = cursor
            .try_collect()
            .await?;

        Ok(results)
    }

    async fn update_document<F, U>(&self, collection: &str, filter: F, update: U) -> OrchestratorResult<bool>
    where
        F: Serialize + Send + Sync,
        U: Serialize + Send + Sync,
    {
        let db = self.get_database()?;
        let coll = db.collection::<Document>(collection);

        let filter_doc = mongodb::bson::to_document(&filter)?;

        let update_doc = mongodb::bson::to_document(&update)?;

        let result = coll.update_one(filter_doc, update_doc, None).await?;

        Ok(result.modified_count > 0)
    }

    async fn delete_document<F>(&self, collection: &str, filter: F) -> OrchestratorResult<bool>
    where
        F: Serialize + Send + Sync,
    {
        let db = self.get_database()?;
        let coll = db.collection::<Document>(collection);

        let filter_doc = mongodb::bson::to_document(&filter)?;

        let result = coll
            .delete_one(filter_doc, None)
            .await?;

        Ok(result.deleted_count > 0)
    }

    async fn count_documents<F>(&self, collection: &str, filter: F) -> OrchestratorResult<u64>
    where
        F: Serialize + Send + Sync,
    {
        let db = self.get_database()?;
        let coll = db.collection::<Document>(collection);

        let filter_doc = mongodb::bson::to_document(&filter)?;

        let count = coll
            .count_documents(filter_doc, None)
            .await?;

        Ok(count)
    }
}
