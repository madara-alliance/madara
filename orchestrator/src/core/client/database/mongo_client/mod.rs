pub mod helpers;

use self::helpers::{record_metrics, ToDocument};
use crate::core::client::database::error::DatabaseError;
use futures::TryStreamExt;
use mongodb::bson::{doc, Document};
use mongodb::options::{FindOneAndUpdateOptions, FindOptions, UpdateOptions};
use mongodb::{bson, Client, Collection, Database, IndexModel};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::sync::Arc;

/// Generic MongoDB client with no business logic knowledge
///
/// This client provides pure CRUD operations without any knowledge of
/// business types like Job, Batch, etc. It handles:
/// - Connection management
/// - Generic CRUD operations
/// - Automatic metrics recording
/// - Index management
pub struct MongoClient {
    client: Client,
    database: Arc<Database>,
}

impl MongoClient {
    /// Create a new MongoClient connection
    pub async fn new(connection_uri: &str, database_name: &str) -> Result<Self, DatabaseError> {
        let client = Client::with_uri_str(connection_uri).await?;
        let database = Arc::new(client.database(database_name));
        Ok(Self { client, database })
    }

    /// Get a typed collection
    pub fn collection<T>(&self, name: &str) -> Collection<T> {
        self.database.collection(name)
    }

    /// Switch to a different database
    pub fn switch_database(&mut self, database_name: &str) {
        self.database = Arc::new(self.client.database(database_name));
    }

    /// Get the underlying MongoDB client (for advanced usage)
    pub fn client(&self) -> &Client {
        &self.client
    }

    // =========================================================================
    // Generic CRUD Operations
    // =========================================================================

    /// Find a single document
    pub async fn find_one<T>(&self, collection: &str, filter: Document) -> Result<Option<T>, DatabaseError>
    where
        T: DeserializeOwned + Unpin + Send + Sync,
    {
        record_metrics("find_one", || async { Ok(self.collection::<T>(collection).find_one(filter, None).await?) })
            .await
    }

    /// Find multiple documents
    pub async fn find_many<T>(
        &self,
        collection: &str,
        filter: Document,
        options: Option<FindOptions>,
    ) -> Result<Vec<T>, DatabaseError>
    where
        T: DeserializeOwned + Unpin + Send + Sync,
    {
        record_metrics("find_many", || async {
            let cursor = self.collection::<T>(collection).find(filter, options).await?;
            Ok(cursor.try_collect().await?)
        })
        .await
    }

    /// Insert a single document
    pub async fn insert_one<T>(&self, collection: &str, doc: T) -> Result<(), DatabaseError>
    where
        T: Serialize + Send + Sync,
    {
        record_metrics("insert_one", || async {
            self.collection::<T>(collection).insert_one(doc, None).await?;
            Ok(())
        })
        .await
    }

    /// Insert if not exists (upsert with $setOnInsert)
    /// Returns true if inserted, false if already existed
    pub async fn insert_if_not_exists<T>(
        &self,
        collection: &str,
        filter: Document,
        doc: T,
    ) -> Result<bool, DatabaseError>
    where
        T: Serialize + ToDocument + Send + Sync,
    {
        record_metrics("insert_if_not_exists", || async {
            let options = UpdateOptions::builder().upsert(true).build();
            let update = doc! { "$setOnInsert": doc.to_document()? };
            let result = self.collection::<T>(collection).update_one(filter, update, options).await?;
            Ok(result.matched_count == 0) // true if inserted, false if existed
        })
        .await
    }

    /// Update a single document
    pub async fn update_one<T>(
        &self,
        collection: &str,
        filter: Document,
        update: Document,
    ) -> Result<UpdateResult, DatabaseError>
    where
        T: Send + Sync,
    {
        record_metrics("update_one", || async {
            let result = self.collection::<T>(collection).update_one(filter, update, None).await?;
            Ok(UpdateResult { matched_count: result.matched_count, modified_count: result.modified_count })
        })
        .await
    }

    /// Update multiple documents
    pub async fn update_many<T>(
        &self,
        collection: &str,
        filter: Document,
        update: Document,
    ) -> Result<UpdateResult, DatabaseError>
    where
        T: Send + Sync,
    {
        record_metrics("update_many", || async {
            let result = self.collection::<T>(collection).update_many(filter, update, None).await?;
            Ok(UpdateResult { matched_count: result.matched_count, modified_count: result.modified_count })
        })
        .await
    }

    /// Find one and update atomically (returns updated document)
    pub async fn find_one_and_update<T>(
        &self,
        collection: &str,
        filter: Document,
        update: Document,
        options: FindOneAndUpdateOptions,
    ) -> Result<Option<T>, DatabaseError>
    where
        T: DeserializeOwned + Unpin + Send + Sync,
    {
        record_metrics("find_one_and_update", || async {
            Ok(self.collection::<T>(collection).find_one_and_update(filter, update, options).await?)
        })
        .await
    }

    /// Count documents matching filter
    pub async fn count<T>(&self, collection: &str, filter: Document) -> Result<u64, DatabaseError>
    where
        T: Send + Sync,
    {
        record_metrics("count", || async { Ok(self.collection::<T>(collection).count_documents(filter, None).await?) })
            .await
    }

    /// Execute aggregation pipeline
    pub async fn aggregate<T, R>(&self, collection: &str, pipeline: Vec<Document>) -> Result<Vec<R>, DatabaseError>
    where
        T: Send + Sync,
        R: DeserializeOwned + Unpin + Send + Sync,
    {
        record_metrics("aggregate", || async {
            let cursor = self.collection::<T>(collection).aggregate(pipeline, None).await?;

            let results: Vec<R> = cursor
                .map_err(DatabaseError::MongoError)
                .and_then(|doc| async move {
                    bson::from_document(doc).map_err(|e| DatabaseError::FailedToSerializeDocument(e.to_string()))
                })
                .try_collect()
                .await?;

            Ok(results)
        })
        .await
    }

    /// Delete a single document
    pub async fn delete_one<T>(&self, collection: &str, filter: Document) -> Result<u64, DatabaseError>
    where
        T: Send + Sync,
    {
        record_metrics("delete_one", || async {
            let result = self.collection::<T>(collection).delete_one(filter, None).await?;
            Ok(result.deleted_count)
        })
        .await
    }

    // =========================================================================
    // Index Management
    // =========================================================================

    /// Create indexes on a collection
    pub async fn create_indexes<T>(&self, collection: &str, indexes: Vec<IndexModel>) -> Result<(), DatabaseError>
    where
        T: Send + Sync,
    {
        record_metrics("create_indexes", || async {
            self.collection::<T>(collection).create_indexes(indexes, None).await?;
            Ok(())
        })
        .await
    }

    // =========================================================================
    // Health Check
    // =========================================================================

    /// Health check - ping the database
    pub async fn health_check(&self) -> Result<(), DatabaseError> {
        record_metrics("health_check", || async {
            self.database.run_command(doc! { "ping": 1 }, None).await?;
            Ok(())
        })
        .await
    }
}

/// Result of an update operation
#[derive(Debug, Clone, Copy)]
pub struct UpdateResult {
    pub matched_count: u64,
    pub modified_count: u64,
}
