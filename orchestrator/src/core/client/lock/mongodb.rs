use crate::core::client::lock::error::LockError;
use crate::core::client::lock::{LockClient, LockInfo, LockResult, LockValue};
use crate::types::params::database::DatabaseArgs;
use async_trait::async_trait;
use chrono::Utc;
use mongodb::bson::doc;
use mongodb::{bson, Client, Collection, Database};
use std::sync::Arc;

/// MongoDB implementation of the CacheService trait
pub struct MongoLockClient {
    client: Client,
    database: Arc<Database>,
    collection_name: String,
}

impl MongoLockClient {
    /// Creates a new MongolockClient instance
    pub async fn new(args: &DatabaseArgs) -> Result<Self, LockError> {
        let client = Client::with_uri_str(&args.connection_uri).await?;
        let database = Arc::new(client.database(&args.database_name));

        Ok(Self { client, database, collection_name: "locks".to_string() })
    }

    /// Creates a new MongolockClient with custom collection name and limits
    pub fn with_config(client: Client, database: Arc<Database>, collection_name: String) -> Self {
        Self { client, database, collection_name }
    }

    // TODO: move this to setup code
    // /// Initialize the cache collection with proper indexes
    // pub async fn initialize(&self) -> Result<(), LockError> {
    //     let start = Instant::now();
    //     let collection = self.get_cache_collection();

    //     // Create indexes for optimal performance
    //     let indexes = vec![
    //         // TTL index on expires_at for automatic cleanup
    //         IndexModel::builder()
    //             .keys(doc! { "expires_at": 1 })
    //             .options(
    //                 IndexOptions::builder()
    //                     .expire_after(std::time::Duration::from_secs(0))
    //                     .name("ttl_index".to_string())
    //                     .build(),
    //             )
    //             .build(),
    //         // Unique index on key for atomic operations
    //         IndexModel::builder()
    //             .keys(doc! { "key": 1 })
    //             .options(IndexOptions::builder().unique(true).name("key_unique_index".to_string()).build())
    //             .build(),
    //         // Index on created_at for analytics
    //         IndexModel::builder()
    //             .keys(doc! { "created_at": 1 })
    //             .options(IndexOptions::builder().name("created_at_index".to_string()).build())
    //             .build(),
    //     ];

    //     if let Err(e) = collection.create_indexes(indexes, None).await {
    //         warn!(
    //             error = %e,
    //             elapsed_ms = %start.elapsed().as_millis(),
    //             "Failed to create cache indexes (they may already exist)"
    //         );
    //     } else {
    //         info!(
    //             elapsed_ms = %start.elapsed().as_millis(),
    //             "Successfully initialized cache collection with indexes"
    //         );
    //     }

    //     Ok(())
    // }

    pub fn get_cache_collection(&self) -> Collection<LockInfo> {
        self.database.collection(&self.collection_name)
    }
}
#[async_trait]
impl LockClient for MongoLockClient {
    async fn acquire_lock_if_available(
        &self,
        key: &str,
        value: LockValue,
        expiry_seconds: u64,
        owner: Option<String>,
    ) -> Result<LockResult, LockError> {
        let collection = self.get_cache_collection();
        let expires_at = Utc::now() + chrono::Duration::seconds(expiry_seconds as i64);

        let lock_info = LockInfo { _id: key.to_string(), value, expires_at: Some(expires_at), owner };

        match collection.insert_one(&lock_info, None).await {
            Ok(_) => Ok(LockResult::Acquired),
            Err(e) => {
                if e.kind.to_string() == "DuplicateKey" {
                    if let Some(existing_lock) = collection.find_one(doc! { "_id": key }, None).await? {
                        return Err(LockError::LockAlreadyHeld { current_owner: existing_lock._id });
                    }
                }
                Ok(LockResult::AlreadyHeld(key.to_string()))
            }
        }
    }

    async fn acquire_lock(
        &self,
        key: &str,
        value: LockValue,
        expiry_seconds: u64,
        owner: Option<String>,
    ) -> Result<LockResult, LockError> {
        match self.acquire_lock_if_available(key, value, expiry_seconds, owner).await? {
            LockResult::Acquired => Ok(LockResult::Acquired),
            LockResult::AlreadyHeld(owner) => Err(LockError::LockAlreadyHeld { current_owner: owner }),
            _ => Err(LockError::InvalidKey(String::from(key))),
        }
    }

    async fn release_lock(&self, key: &str, owner: Option<&str>) -> Result<LockResult, LockError> {
        let collection = self.get_cache_collection();

        let filter = match owner {
            Some(owner) => doc! { "_id": key, "owner": { "$eq": owner } },
            None => doc! { "_id": key },
        };

        match collection.delete_one(filter, None).await {
            Ok(result) => {
                if result.deleted_count > 0 {
                    Ok(LockResult::Released)
                } else {
                    Err(LockError::KeyNotFound(key.to_string()))
                }
            }
            Err(e) => Err(LockError::MongoDB(e)),
        }
    }

    async fn extend_lock(
        &self,
        key: &str,
        value: LockValue,
        expiry_seconds: u64,
        owner: Option<String>,
    ) -> Result<LockResult, LockError> {
        let collection = self.get_cache_collection();
        let expires_at = Utc::now() + chrono::Duration::seconds(expiry_seconds as i64);

        let update = doc! {
            "$set": {
                "value": bson::to_bson(&value)?,
                "expires_at": expires_at
            }
        };
        let filter = match owner {
            Some(owner) => doc! { "_id": key, "owner": { "$eq": owner } },
            None => doc! { "_id": key },
        };

        match collection.update_one(filter, update, None).await {
            Ok(result) => {
                if result.modified_count > 0 {
                    Ok(LockResult::Extended)
                } else {
                    Ok(LockResult::NotFound)
                }
            }
            Err(e) => Err(LockError::MongoDB(e)),
        }
    }

    async fn get_lock(&self, key: &str, owner: Option<String>) -> Result<LockInfo, LockError> {
        let collection = self.get_cache_collection();
        let filter = match owner {
            Some(owner) => doc! { "_id": key, "owner": { "$eq": owner } },
            None => doc! { "_id": key },
        };

        match collection.find_one(filter, None).await {
            Ok(Some(lock)) => Ok(lock),
            Err(e) => Err(LockError::MongoDB(e)),
            _ => Err(LockError::InvalidKey(String::from(key))),
        }
    }

    async fn is_locked(&self, key: &str) -> Result<bool, LockError> {
        let collection = self.get_cache_collection();

        match collection
            .find_one(
                doc! {
                    "_id": key,
                    "expires_at": { "$gt": Utc::now() }
                },
                None,
            )
            .await
        {
            Ok(Some(_)) => Ok(true),
            Ok(None) => Ok(false),
            Err(e) => Err(LockError::MongoDB(e)),
        }
    }

    async fn get_lock_ttl(&self, key: &str) -> Result<Option<i64>, LockError> {
        let collection = self.get_cache_collection();

        match collection.find_one(doc! { "_id": key }, None).await {
            Ok(Some(lock)) => {
                if let Some(expires_at) = lock.expires_at {
                    let now = Utc::now();
                    if expires_at > now {
                        Ok(Some((expires_at - now).num_seconds()))
                    } else {
                        Ok(Some(0))
                    }
                } else {
                    Ok(None)
                }
            }
            Ok(None) => Ok(None),
            Err(e) => Err(LockError::MongoDB(e)),
        }
    }
}
