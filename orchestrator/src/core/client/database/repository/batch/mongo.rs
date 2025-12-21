use async_trait::async_trait;
use chrono::{SubsecRound, Utc};
use mongodb::bson::{doc, Bson, Document};
use mongodb::options::{FindOneAndUpdateOptions, FindOptions, ReturnDocument};
use std::sync::Arc;
use tracing::debug;

use super::r#trait::BatchRepository;
use crate::core::client::database::constant::{
    AGGREGATOR_BATCHES_COLLECTION, JOBS_COLLECTION, SNOS_BATCHES_COLLECTION,
};
use crate::core::client::database::error::DatabaseError;
use crate::core::client::database::mongo_client::helpers::ToDocument;
use crate::core::client::database::mongo_client::MongoClient;
use crate::types::batch::{
    AggregatorBatch, AggregatorBatchStatus, AggregatorBatchUpdates, SnosBatch, SnosBatchStatus, SnosBatchUpdates,
};
use crate::types::jobs::types::JobType;

pub struct MongoBatchRepository {
    client: Arc<MongoClient>,
}

impl MongoBatchRepository {
    pub fn new(client: Arc<MongoClient>) -> Self {
        Self { client }
    }

    /// Helper to build update document from batch + updates
    fn build_upsert_doc<T: ToDocument>(
        &self,
        batch: &T,
        updates: &impl ToDocument,
        end_block: Option<u64>,
        start_block: u64,
    ) -> Result<Document, DatabaseError> {
        let mut non_null = Document::new();

        // Add non-null fields from batch
        for (k, v) in batch.to_document()? {
            if v != Bson::Null {
                non_null.insert(k, v);
            }
        }

        // Add non-null fields from updates (overwrites batch fields)
        for (k, v) in updates.to_document()? {
            if v != Bson::Null {
                non_null.insert(k, v);
            }
        }

        // Calculate num_blocks if end_block provided
        if let Some(end) = end_block {
            non_null.insert("num_blocks", Bson::Int64(end as i64 - start_block as i64 + 1));
        }

        non_null.insert("updated_at", Bson::DateTime(Utc::now().round_subsecs(0).into()));

        Ok(doc! { "$set": non_null })
    }
}

#[async_trait]
impl BatchRepository for MongoBatchRepository {
    // =========================================================================
    // SNOS Batch Operations
    // =========================================================================

    async fn create_snos_batch(&self, batch: SnosBatch) -> Result<SnosBatch, DatabaseError> {
        self.client.insert_one(SNOS_BATCHES_COLLECTION, batch.clone()).await?;
        debug!(batch_id = %batch.id, index = batch.index, "SNOS batch created");
        Ok(batch)
    }

    async fn get_latest_snos_batch(&self) -> Result<Option<SnosBatch>, DatabaseError> {
        let options = FindOptions::builder().sort(doc! { "index": -1 }).limit(1).build();

        let mut results: Vec<SnosBatch> =
            self.client.find_many(SNOS_BATCHES_COLLECTION, doc! {}, Some(options)).await?;

        Ok(results.pop())
    }

    async fn get_snos_batches_by_indices(&self, indices: Vec<u64>) -> Result<Vec<SnosBatch>, DatabaseError> {
        let indices_bson: Vec<Bson> = indices.iter().map(|i| Bson::Int64(*i as i64)).collect();

        let filter = doc! { "index": { "$in": indices_bson } };
        self.client.find_many(SNOS_BATCHES_COLLECTION, filter, None).await
    }

    async fn get_snos_batches_by_status(
        &self,
        status: SnosBatchStatus,
        limit: Option<i64>,
    ) -> Result<Vec<SnosBatch>, DatabaseError> {
        let filter = doc! { "status": status.to_string() };
        let options = FindOptions::builder().sort(doc! { "index": 1 }).limit(limit).build();

        self.client.find_many(SNOS_BATCHES_COLLECTION, filter, Some(options)).await
    }

    async fn get_snos_batches_without_jobs(&self, status: SnosBatchStatus) -> Result<Vec<SnosBatch>, DatabaseError> {
        let pipeline = vec![
            doc! { "$match": { "status": status.to_string() } },
            doc! {
                "$lookup": {
                    "from": JOBS_COLLECTION,
                    "let": { "index": { "$toString": "$index" } },
                    "pipeline": [
                        {
                            "$match": {
                                "$expr": {
                                    "$and": [
                                        { "$eq": ["$job_type", format!("{:?}", JobType::SnosRun)] },
                                        { "$eq": ["$internal_id", "$$index"] }
                                    ]
                                }
                            }
                        }
                    ],
                    "as": "corresponding_jobs"
                }
            },
            doc! { "$match": { "corresponding_jobs": { "$eq": [] } } },
            doc! { "$sort": { "index": 1 } },
        ];

        self.client.aggregate::<SnosBatch, SnosBatch>(SNOS_BATCHES_COLLECTION, pipeline).await
    }

    async fn update_snos_batch_status(&self, index: u64, status: SnosBatchStatus) -> Result<SnosBatch, DatabaseError> {
        let filter = doc! { "index": index as i64 };
        let update = doc! {
            "$set": {
                "status": status.to_string(),
                "updated_at": Bson::DateTime(Utc::now().round_subsecs(0).into()),
            }
        };

        let options = FindOneAndUpdateOptions::builder().return_document(ReturnDocument::After).build();

        self.client
            .find_one_and_update::<SnosBatch>(SNOS_BATCHES_COLLECTION, filter, update, options)
            .await?
            .ok_or_else(|| DatabaseError::UpdateFailed(format!("SNOS batch {} not found", index)))
    }

    async fn upsert_snos_batch(
        &self,
        batch: &SnosBatch,
        update: &SnosBatchUpdates,
    ) -> Result<SnosBatch, DatabaseError> {
        let filter = doc! { "_id": batch.id };
        let update_doc = self.build_upsert_doc(batch, update, update.end_block, batch.start_block)?;

        let options = FindOneAndUpdateOptions::builder().upsert(true).return_document(ReturnDocument::After).build();

        self.client
            .find_one_and_update::<SnosBatch>(SNOS_BATCHES_COLLECTION, filter, update_doc, options)
            .await?
            .ok_or_else(|| DatabaseError::UpdateFailed(format!("SNOS batch {} upsert failed", batch.id)))
    }

    async fn get_snos_batches_by_aggregator(&self, aggregator_index: u64) -> Result<Vec<SnosBatch>, DatabaseError> {
        let filter = doc! { "aggregator_batch_index": aggregator_index as i64 };
        let options = FindOptions::builder().sort(doc! { "index": 1 }).build();

        self.client.find_many(SNOS_BATCHES_COLLECTION, filter, Some(options)).await
    }

    async fn get_open_snos_batches_by_aggregator(
        &self,
        aggregator_index: u64,
    ) -> Result<Vec<SnosBatch>, DatabaseError> {
        let filter = doc! {
            "aggregator_batch_index": aggregator_index as i64,
            "status": "Open",
        };

        self.client.find_many(SNOS_BATCHES_COLLECTION, filter, None).await
    }

    async fn get_first_snos_batch_for_aggregator(
        &self,
        aggregator_index: u64,
    ) -> Result<Option<SnosBatch>, DatabaseError> {
        let pipeline = vec![
            doc! { "$match": { "aggregator_batch_index": aggregator_index as i64 } },
            doc! { "$sort": { "index": 1 } },
            doc! { "$limit": 1 },
        ];

        let mut results: Vec<SnosBatch> =
            self.client.aggregate::<SnosBatch, SnosBatch>(SNOS_BATCHES_COLLECTION, pipeline).await?;

        Ok(results.pop())
    }

    async fn count_snos_batches_by_aggregator(&self, aggregator_index: u64) -> Result<u64, DatabaseError> {
        let filter = doc! { "aggregator_batch_index": aggregator_index as i64 };
        self.client.count::<SnosBatch>(SNOS_BATCHES_COLLECTION, filter).await
    }

    async fn get_next_snos_batch_id(&self) -> Result<u64, DatabaseError> {
        let latest = self.get_latest_snos_batch().await?;
        Ok(latest.map_or(1, |b| b.index + 1))
    }

    async fn close_snos_batches_for_aggregator(&self, aggregator_index: u64) -> Result<Vec<SnosBatch>, DatabaseError> {
        let filter = doc! {
            "aggregator_batch_index": aggregator_index as i64,
            "status": { "$ne": "Closed" },
        };
        let update = doc! {
            "$set": {
                "status": "Closed",
                "updated_at": Bson::DateTime(Utc::now().round_subsecs(0).into()),
            }
        };

        self.client.update_many::<SnosBatch>(SNOS_BATCHES_COLLECTION, filter, update).await?;

        // Return all closed batches for this aggregator
        let closed_filter = doc! {
            "aggregator_batch_index": aggregator_index as i64,
            "status": "Closed",
        };
        self.client.find_many(SNOS_BATCHES_COLLECTION, closed_filter, None).await
    }

    // =========================================================================
    // Aggregator Batch Operations
    // =========================================================================

    async fn create_aggregator_batch(&self, batch: AggregatorBatch) -> Result<AggregatorBatch, DatabaseError> {
        self.client.insert_one(AGGREGATOR_BATCHES_COLLECTION, batch.clone()).await?;
        debug!(batch_id = %batch.id, index = batch.index, "Aggregator batch created");
        Ok(batch)
    }

    async fn get_latest_aggregator_batch(&self) -> Result<Option<AggregatorBatch>, DatabaseError> {
        let options = FindOptions::builder().sort(doc! { "index": -1 }).limit(1).build();

        let mut results: Vec<AggregatorBatch> =
            self.client.find_many(AGGREGATOR_BATCHES_COLLECTION, doc! {}, Some(options)).await?;

        Ok(results.pop())
    }

    async fn get_aggregator_batches_by_indices(
        &self,
        indices: Vec<u64>,
    ) -> Result<Vec<AggregatorBatch>, DatabaseError> {
        let indices_bson: Vec<Bson> = indices.iter().map(|i| Bson::Int64(*i as i64)).collect();

        let filter = doc! { "index": { "$in": indices_bson } };
        self.client.find_many(AGGREGATOR_BATCHES_COLLECTION, filter, None).await
    }

    async fn get_aggregator_batches_by_status(
        &self,
        status: AggregatorBatchStatus,
        limit: Option<i64>,
    ) -> Result<Vec<AggregatorBatch>, DatabaseError> {
        let filter = doc! { "status": status.to_string() };
        let options = FindOptions::builder().sort(doc! { "index": 1 }).limit(limit).build();

        self.client.find_many(AGGREGATOR_BATCHES_COLLECTION, filter, Some(options)).await
    }

    async fn get_aggregator_batch_for_block(
        &self,
        block_number: u64,
    ) -> Result<Option<AggregatorBatch>, DatabaseError> {
        let block = block_number as i64;
        let filter = doc! {
            "start_block": { "$lte": block },
            "end_block": { "$gte": block },
        };

        self.client.find_one(AGGREGATOR_BATCHES_COLLECTION, filter).await
    }

    async fn update_aggregator_batch_status(
        &self,
        index: u64,
        status: AggregatorBatchStatus,
    ) -> Result<AggregatorBatch, DatabaseError> {
        let filter = doc! { "index": index as i64 };
        let update = doc! {
            "$set": {
                "status": status.to_string(),
                "updated_at": Bson::DateTime(Utc::now().round_subsecs(0).into()),
            }
        };

        let options = FindOneAndUpdateOptions::builder().return_document(ReturnDocument::After).build();

        self.client
            .find_one_and_update::<AggregatorBatch>(AGGREGATOR_BATCHES_COLLECTION, filter, update, options)
            .await?
            .ok_or_else(|| DatabaseError::UpdateFailed(format!("Aggregator batch {} not found", index)))
    }

    async fn upsert_aggregator_batch(
        &self,
        batch: &AggregatorBatch,
        update: &AggregatorBatchUpdates,
    ) -> Result<AggregatorBatch, DatabaseError> {
        let filter = doc! { "_id": batch.id };
        let update_doc = self.build_upsert_doc(batch, update, update.end_block, batch.start_block)?;

        let options = FindOneAndUpdateOptions::builder().upsert(true).return_document(ReturnDocument::After).build();

        self.client
            .find_one_and_update::<AggregatorBatch>(AGGREGATOR_BATCHES_COLLECTION, filter, update_doc, options)
            .await?
            .ok_or_else(|| DatabaseError::UpdateFailed(format!("Aggregator batch {} upsert failed", batch.id)))
    }
}
