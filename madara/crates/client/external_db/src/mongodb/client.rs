//! MongoDB connection management.

use crate::{config::ExternalDbConfig, mongodb::MempoolTransactionDocument};
use anyhow::Result;
use mongodb::{bson::doc, options::ClientOptions};
use std::time::Duration;

/// MongoDB client wrapper for external database operations.
pub struct MongoClient {
    client: mongodb::Client,
    collection: mongodb::Collection<MempoolTransactionDocument>,
}

impl MongoClient {
    /// Creates a new MongoDB client.
    pub async fn new(config: &ExternalDbConfig) -> Result<Self> {
        let mut options = ClientOptions::parse(&config.mongodb_uri).await?;
        options.max_pool_size = Some(config.pool_size);
        options.min_pool_size = Some(1);
        options.connect_timeout = Some(Duration::from_secs(10));
        options.server_selection_timeout = Some(Duration::from_secs(10));

        let client = mongodb::Client::with_options(options)?;
        client.database("admin").run_command(doc! { "ping": 1 }).await?;

        let db = client.database(&config.database_name);
        let collection = db.collection::<MempoolTransactionDocument>(&config.collection_name);

        Ok(Self { client, collection })
    }

    pub fn collection(&self) -> &mongodb::Collection<MempoolTransactionDocument> {
        &self.collection
    }

    pub fn client(&self) -> &mongodb::Client {
        &self.client
    }
}
