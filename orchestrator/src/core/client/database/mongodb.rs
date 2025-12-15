use super::error::DatabaseError;
use crate::core::client::database::constant::{
    AGGREGATOR_BATCHES_COLLECTION, JOBS_COLLECTION, SNOS_BATCHES_COLLECTION,
};
use crate::core::client::database::DatabaseClient;
use crate::core::client::lock::constant::LOCKS_COLLECTION;
use crate::types::batch::{
    AggregatorBatch, AggregatorBatchStatus, AggregatorBatchUpdates, SnosBatch, SnosBatchStatus, SnosBatchUpdates,
};
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::job_updates::JobItemUpdates;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::types::params::database::DatabaseArgs;
use crate::utils::metrics::ORCHESTRATOR_METRICS;
use async_trait::async_trait;
use chrono::{SubsecRound, Utc};
use futures::TryStreamExt;
use mongodb::bson::{doc, Bson, Document};
use mongodb::options::{
    AggregateOptions, FindOneAndUpdateOptions, FindOptions, InsertOneOptions, ReturnDocument, UpdateOptions,
};
use mongodb::{bson, Client, Collection, Database};
use opentelemetry::KeyValue;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, error, warn};
use uuid::Uuid;

pub trait ToDocument {
    fn to_document(&self) -> Result<Document, DatabaseError>;
}

impl<T: Serialize> ToDocument for T {
    fn to_document(&self) -> Result<Document, DatabaseError> {
        let doc = bson::to_bson(self)?;

        if let Bson::Document(doc) = doc {
            Ok(doc)
        } else {
            Err(DatabaseError::FailedToSerializeDocument(format!("Failed to serialize document: {}", doc)))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateResult {
    pub matched_count: u64,
    pub modified_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteResult {
    pub deleted_count: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct MissingBlocksResponse {
    pub missing_blocks: Vec<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct MissingBatchIndicesResponse {
    pub missing_batch_indices: Vec<u64>,
}

/// MongoDB client implementation
pub struct MongoDbClient {
    client: Client,
    database: Arc<Database>,
}

impl MongoDbClient {
    pub async fn new(config: &DatabaseArgs) -> Result<Self, DatabaseError> {
        let client = Client::with_uri_str(&config.connection_uri).await?;
        let database = Arc::new(client.database(&config.database_name));
        Ok(Self { client, database })
    }

    /// Mongodb client uses Arc internally, reducing the cost of clone.
    /// Directly using clone is not recommended for libraries not using Arc internally.
    pub fn client(&self) -> Client {
        self.client.clone()
    }

    fn get_job_collection(&self) -> Collection<JobItem> {
        self.database.collection(JOBS_COLLECTION)
    }

    /// Get the MongoDB collection for aggregator batches
    ///
    /// Returns a typed collection interface for performing operations on aggregator batches.
    /// Uses the dedicated `aggregator_batches` collection for better data organization.
    fn get_aggregator_batch_collection(&self) -> Collection<AggregatorBatch> {
        self.database.collection(AGGREGATOR_BATCHES_COLLECTION)
    }

    /// Get the MongoDB collection for SNOS batches
    ///
    /// Returns a typed collection interface for performing operations on SNOS batches.
    /// Uses the dedicated `snos_batches` collection for better data organization.
    fn get_snos_batch_collection(&self) -> Collection<SnosBatch> {
        self.database.collection(SNOS_BATCHES_COLLECTION)
    }

    pub fn get_collection<T>(&self, name: &str) -> Collection<T> {
        self.database.collection(name)
    }

    pub fn jobs_collection(&self) -> Collection<JobItem> {
        self.get_collection::<JobItem>(JOBS_COLLECTION)
    }

    pub fn locks_collection(&self) -> Collection<JobItem> {
        self.get_collection(LOCKS_COLLECTION)
    }

    /// find_one - Find one document in a collection
    /// # Arguments
    /// * `collection` - The collection to find the document in
    /// * `filter` - The filter to apply to the collection
    /// # Returns
    /// * `Result<Option<T>, DatabaseError>` - A Result indicating whether the operation was successful or not
    pub async fn find_one<T>(&self, collection: Collection<T>, filter: Document) -> Result<Option<T>, DatabaseError>
    where
        T: DeserializeOwned + Unpin + Send + Sync + Sized,
    {
        Ok(collection.find_one(filter, None).await?)
    }

    /// update_one - Update one document in a collection
    /// # Arguments
    /// * `collection` - The collection to update the document in
    /// * `filter` - The filter to apply to the collection
    /// * `update` - The update to apply to the document
    /// * `options` - The options to apply to the update
    /// # Returns
    /// * `Result<UpdateResult, DatabaseError>` - A Result indicating whether the operation was successful or not
    pub async fn update_one<T>(
        &self,
        collection: Collection<T>,
        filter: Document,
        update: Document,
        options: Option<UpdateOptions>,
    ) -> Result<UpdateResult, DatabaseError>
    where
        T: Serialize + Sized,
    {
        let result = collection.update_one(filter, update, options).await?;
        Ok(UpdateResult { matched_count: result.matched_count, modified_count: result.modified_count })
    }

    /// delete_one - Delete one document in a collection
    /// # Arguments
    /// * `collection` - The collection to delete the document in
    /// * `filter` - The filter to apply to the collection
    /// # Returns
    /// * `Result<DeleteResult, DatabaseError>` - A Result indicating whether the operation was successful or not
    pub async fn delete_one<T>(
        &self,
        collection: Collection<T>,
        filter: Document,
    ) -> Result<DeleteResult, DatabaseError>
    where
        T: Serialize + Sized,
    {
        let result = collection.delete_one(filter, None).await?;
        Ok(DeleteResult { deleted_count: result.deleted_count })
    }

    /// find - Find multiple documents in a collection
    /// # Arguments
    /// * `collection` - The collection to find the documents in
    /// * `filter` - The filter to apply to the collection
    /// * `sort` - The sort to apply to the collection
    /// * `limit` - The limit to apply to the collection
    /// * `skip` - The skip to apply to the collection
    /// * `projection` - The projection to apply to the collection
    /// # Returns
    pub async fn find<T>(
        &self,
        collection: Collection<T>,
        filter: Document,
        sort: Option<Document>,
        limit: Option<i64>,
        skip: Option<i64>,
        projection: Option<Document>,
    ) -> Result<Vec<T>, DatabaseError>
    where
        T: DeserializeOwned + Unpin + Send + Sync + Sized,
    {
        let start = Instant::now();
        let mut pipeline = vec![doc! {
            "$match": filter
        }];
        if let Some(sort) = sort {
            pipeline.push(doc! {
                "$sort": sort
            });
        }
        if let Some(limit) = limit {
            pipeline.push(doc! {
                "$limit": limit
            });
        }
        if let Some(skip) = skip {
            pipeline.push(doc! {
                "$skip": skip
            });
        }
        if let Some(projection) = projection {
            pipeline.push(doc! {
                "$project": projection
            });
        }

        let cursor = collection.aggregate(pipeline, None).await?;
        let vec_items: Vec<T> = cursor
            .map_err(|e| {
                error!(error = %e, "Error retrieving document");
                DatabaseError::FailedToSerializeDocument(format!("Failed to retrieve document: {}", e))
            })
            .and_then(|doc| {
                futures::future::ready(bson::from_document::<T>(doc).map_err(|e| {
                    error!(error = %e, "Deserialization error");
                    DatabaseError::FailedToSerializeDocument(format!("Failed to deserialize document: {}", e))
                }))
            })
            .try_collect()
            .await?;
        debug!("Fetched data from collection");
        let attributes = [KeyValue::new("db_operation_name", "find")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
        Ok(vec_items)
    }

    /// execute_pipeline - Execute a custom aggregation pipeline on a collection
    /// # Arguments
    /// * `collection` - The collection to execute the pipeline on
    /// * `pipeline` - The aggregation pipeline to execute
    /// * `options` - Optional aggregation options
    /// # Returns
    /// * `Result<Vec<R>, DatabaseError>` - A Result containing the pipeline results or an error
    pub async fn execute_pipeline<T, R>(
        &self,
        collection: Collection<T>,
        pipeline: Vec<Document>,
        options: Option<AggregateOptions>,
    ) -> Result<Vec<R>, DatabaseError>
    where
        T: DeserializeOwned + Unpin + Send + Sync + Sized,
        R: DeserializeOwned + Unpin + Send + Sync + Sized,
    {
        let start = Instant::now();

        debug!("Executing aggregation pipeline");

        let cursor = collection.aggregate(pipeline, options).await?;
        let vec_items: Vec<R> = cursor
            .map_err(|e| {
                error!(error = %e, "Error executing pipeline");
                DatabaseError::FailedToSerializeDocument(format!("Failed to execute pipeline: {}", e))
            })
            .and_then(|doc| {
                futures::future::ready(bson::from_document::<R>(doc).map_err(|e| {
                    error!(error = %e, "Deserialization error");
                    DatabaseError::FailedToSerializeDocument(format!("Failed to deserialize: {}", e))
                }))
            })
            .try_collect()
            .await?;

        debug!(result_count = vec_items.len(), "Pipeline execution completed");

        let duration = start.elapsed();
        let attrs = [KeyValue::new("db_operation_name", "execute_pipeline")];
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attrs);

        Ok(vec_items)
    }
}

#[async_trait]
impl DatabaseClient for MongoDbClient {
    async fn switch_database(&mut self, database_name: &str) -> Result<(), DatabaseError> {
        self.database = Arc::new(self.client.database(database_name));
        Ok(())
    }

    async fn disconnect(&self) -> Result<(), DatabaseError> {
        // MongoDB client automatically disconnects when dropped
        Ok(())
    }

    /// create_job - Create a new job in the database
    /// This function creates a new job in the database
    /// It returns a Result<JobItem, DatabaseError> indicating whether the operation was successful or not
    /// # Arguments
    /// * `job` - The job to be created
    /// # Returns
    /// * `Result<JobItem, DatabaseError>` - A Result indicating whether the operation was successful or not
    async fn create_job(&self, job: JobItem) -> Result<JobItem, DatabaseError> {
        let start = Instant::now();
        let options = UpdateOptions::builder().upsert(true).build();

        let updates = job.to_document()?;
        let job_type = updates.get("job_type").ok_or(DatabaseError::KeyNotFound("job_type".to_string()))?;
        let internal_id = updates.get("internal_id").ok_or(DatabaseError::KeyNotFound("internal_id".to_string()))?;

        // Filter using only two fields
        let filter = doc! {
            "job_type": job_type.clone(),
            "internal_id": internal_id.clone()
        };
        let updates = doc! {
            // only set when the document is inserted for the first time
            "$setOnInsert": updates
        };

        let result = self.get_job_collection().update_one(filter, updates, options).await?;

        if result.matched_count == 0 {
            let duration = start.elapsed();
            debug!(duration = %duration.as_millis(), "Job created in MongoDB successfully");

            let attributes = [KeyValue::new("db_operation_name", "create_job")];
            ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
            Ok(job)
        } else {
            Err(DatabaseError::ItemAlreadyExists(format!(
                "Job already exists for internal_id {} and job_type {:?}",
                job.internal_id, job.job_type
            )))
        }
    }

    async fn get_job_by_id(&self, id: Uuid) -> Result<Option<JobItem>, DatabaseError> {
        let start = Instant::now();
        let filter = doc! {
            "id":  id
        };
        debug!("Fetched job by ID");
        let attributes = [KeyValue::new("db_operation_name", "get_job_by_id")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
        Ok(self.get_job_collection().find_one(filter, None).await?)
    }

    async fn get_job_by_internal_id_and_type(
        &self,
        internal_id: &str,
        job_type: &JobType,
    ) -> Result<Option<JobItem>, DatabaseError> {
        let start = Instant::now();
        let filter = doc! {
            "internal_id": internal_id,
            "job_type": bson::to_bson(&job_type)?,
        };
        debug!("Fetched job by internal ID and type");
        let attributes = [KeyValue::new("db_operation_name", "get_job_by_internal_id_and_type")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
        Ok(self.get_job_collection().find_one(filter, None).await?)
    }

    async fn update_job(&self, current_job: &JobItem, update: JobItemUpdates) -> Result<JobItem, DatabaseError> {
        let start = Instant::now();
        // Filters to search for the job
        let filter = doc! {
            "id": current_job.id,
            "version": current_job.version,
        };
        let options = FindOneAndUpdateOptions::builder().upsert(false).return_document(ReturnDocument::After).build();

        let mut updates = update.to_document()?;

        // remove null values from the updates
        let mut non_null_updates = Document::new();
        updates.iter_mut().for_each(|(k, v)| {
            if v != &Bson::Null {
                non_null_updates.insert(k, v);
            }
        });

        // throw an error if there's no field to be updated
        if non_null_updates.is_empty() {
            return Err(DatabaseError::NoUpdateFound("No field to be updated, likely a false call".to_string()));
        }

        // Add additional fields that are always updated
        non_null_updates.insert("version", Bson::Int32(current_job.version + 1));
        non_null_updates.insert("updated_at", Bson::DateTime(Utc::now().round_subsecs(0).into()));

        let update = doc! {
            "$set": non_null_updates
        };

        let result = self.get_job_collection().find_one_and_update(filter, update, options).await?;
        match result {
            Some(job) => {
                debug!("Job updated successfully");
                let attributes = [KeyValue::new("db_operation_name", "update_job")];
                let duration = start.elapsed();
                ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
                Ok(job)
            }
            None => {
                warn!(version = %current_job.version, "Failed to update job. Job version is likely outdated");
                Err(DatabaseError::UpdateFailed(format!("Failed to update job. Identifier - {}, ", current_job.id)))
            }
        }
    }

    async fn get_latest_job_by_type(&self, job_type: JobType) -> Result<Option<JobItem>, DatabaseError> {
        let start = Instant::now();
        let pipeline = vec![
            doc! {
                "$match": {
                    "job_type": bson::to_bson(&job_type)?
                }
            },
            doc! {
                "$addFields": {
                    "numeric_internal_id": { "$toLong": "$internal_id" }
                }
            },
            doc! {
                "$sort": {
                    "numeric_internal_id": -1
                }
            },
            doc! {
                "$limit": 1
            },
            doc! {
                "$project": {
                    "numeric_internal_id": 0  // Remove the temporary field
                }
            },
        ];

        debug!("Fetching latest job by type");

        let results = self.execute_pipeline::<JobItem, JobItem>(self.get_job_collection(), pipeline, None).await?;

        let attributes = [KeyValue::new("db_operation_name", "get_latest_job_by_type")];
        let duration = start.elapsed();

        let result = vec_to_single_result(results, "get_latest_job_by_type")?;

        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
        Ok(result)
    }

    /// Function to get jobs that don't have a successor job.
    ///
    /// `job_a_type`: Type of job that we need to get that doesn't have any successor.
    ///
    /// `job_a_status`: Status of job A.
    ///
    /// `job_b_type`: Type of job that we need to have as a successor for Job A.
    ///
    /// `job_b_status`: Status of job B which we want to check with.
    ///
    /// Example use case:
    /// Getting SNOS jobs that do not have a successive proving job initiated yet.
    ///
    /// # Arguments
    /// `job_a_type`: SnosRun
    /// `job_a_status`: Completed
    /// `job_b_type`: ProofCreation
    async fn get_jobs_without_successor(
        &self,
        job_a_type: JobType,
        job_a_status: JobStatus,
        job_b_type: JobType,
    ) -> Result<Vec<JobItem>, DatabaseError> {
        let start = Instant::now();
        // Convert enums to Bson strings
        let job_a_type_bson = Bson::String(format!("{:?}", job_a_type));
        let job_a_status_bson = Bson::String(format!("{:?}", job_a_status));
        let job_b_type_bson = Bson::String(format!("{:?}", job_b_type));

        // Construct the aggregation pipeline
        let pipeline = vec![
            // Stage 1: Match job_a_type with job_a_status
            doc! {
                "$match": {
                    "job_type": job_a_type_bson,
                    "status": job_a_status_bson,
                }
            },
            // Stage 2: Lookup to find corresponding job_b_type jobs
            doc! {
                "$lookup": {
                    "from": JOBS_COLLECTION,
                    "let": { "internal_id": "$internal_id" },
                    "pipeline": [
                        {
                            "$match": {
                                "$expr": {
                                    "$and": [
                                        { "$eq": ["$job_type", job_b_type_bson] },
                                        { "$eq": ["$internal_id", "$$internal_id"] }
                                    ]
                                }
                            }
                        }
                    ],
                    "as": "successor_jobs"
                }
            },
            // Stage 3: Filter out job_a_type jobs that have corresponding job_b_type jobs
            doc! {
                "$match": {
                    "successor_jobs": { "$eq": [] }
                }
            },
        ];

        debug!("Fetching jobs without successor");

        let result = self.execute_pipeline::<JobItem, JobItem>(self.get_job_collection(), pipeline, None).await?;

        debug!(job_count = result.len(), "Retrieved jobs without successor");
        let attributes = [KeyValue::new("db_operation_name", "get_jobs_without_successor")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);

        Ok(result)
    }

    async fn get_jobs_after_internal_id_by_job_type(
        &self,
        job_type: JobType,
        job_status: JobStatus,
        internal_id: String,
    ) -> Result<Vec<JobItem>, DatabaseError> {
        let start = Instant::now();
        let filter = doc! {
            "job_type": bson::to_bson(&job_type)?,
            "status": bson::to_bson(&job_status)?,
            "$expr": {
                "$gt": [
                    { "$toInt": "$internal_id" },  // Convert stored string to number
                    { "$toInt": &internal_id }     // Convert input string to number
                ]
            }
        };
        let jobs: Vec<JobItem> = self.get_job_collection().find(filter, None).await?.try_collect().await?;
        debug!("Fetched jobs after internal ID by job type");
        let attributes = [KeyValue::new("db_operation_name", "get_jobs_after_internal_id_by_job_type")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
        Ok(jobs)
    }

    async fn get_jobs_by_types_and_statuses(
        &self,
        job_type: Vec<JobType>,
        job_status: Vec<JobStatus>,
        limit: Option<i64>,
    ) -> Result<Vec<JobItem>, DatabaseError> {
        let start = Instant::now();

        let mut filter = doc! {};

        // Only add job_type filter if the vector is not empty
        if !job_type.is_empty() {
            let serialized_job_type: Result<Vec<Bson>, _> = job_type.iter().map(bson::to_bson).collect();
            filter.insert("job_type", doc! { "$in": serialized_job_type? });
        }

        // Only add the status filter if the vector is not empty
        if !job_status.is_empty() {
            let serialized_statuses: Result<Vec<Bson>, _> = job_status.iter().map(bson::to_bson).collect();
            filter.insert("status", doc! { "$in": serialized_statuses? });
        }

        let find_options = limit.map(|val| FindOptions::builder().limit(Some(val)).build());

        let jobs: Vec<JobItem> = self.get_job_collection().find(filter, find_options).await?.try_collect().await?;
        debug!(job_count = jobs.len(), "Retrieved jobs by type and statuses");
        let attributes = [KeyValue::new("db_operation_name", "get_jobs_by_types_and_status")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
        Ok(jobs)
    }

    async fn get_latest_job_by_type_and_status(
        &self,
        job_type: JobType,
        job_status: JobStatus,
    ) -> Result<Option<JobItem>, DatabaseError> {
        let start = Instant::now();

        // Convert job_type to Bson
        let job_type_bson = bson::to_bson(&job_type)?;
        let status_bson = bson::to_bson(&job_status)?;

        // Construct the aggregation pipeline
        let pipeline = vec![
            // Stage 1: Match by type + status
            doc! {
                "$match": {
                    "job_type": job_type_bson,
                    "status": status_bson,
                }
            },
            // Stage 2: Sort by block_number descending
            doc! {
                "$sort": {
                    "metadata.specific.block_number": -1
                }
            },
            // Stage 3: Take only the top document
            doc! { "$limit": 1 },
        ];

        debug!("Fetching latest job by type and status");

        let collection: Collection<JobItem> = self.get_job_collection();

        // Execute pipeline and convert Vec<JobItem> to Option<JobItem>
        let results = self.execute_pipeline::<JobItem, JobItem>(collection, pipeline, None).await?;

        let attributes = [KeyValue::new("db_operation_name", "get_latest_job_by_type_and_status")];

        let result = vec_to_single_result(results, "get_latest_job_by_type_and_status")?;

        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
        Ok(result)
    }

    async fn get_latest_aggregator_batch(&self) -> Result<Option<AggregatorBatch>, DatabaseError> {
        let start = Instant::now();
        let options = FindOptions::builder().sort(doc! { "index": -1 }).limit(1).build();

        let mut cursor = self.get_aggregator_batch_collection().find(doc! {}, options).await?;
        let batch = cursor.try_next().await?;

        debug!(has_batch = batch.is_some(), category = "db_call", "Retrieved latest aggregator batch");

        let attributes = [KeyValue::new("db_operation_name", "get_latest_aggregator_batch")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);

        Ok(batch)
    }

    async fn get_latest_snos_batch(&self) -> Result<Option<SnosBatch>, DatabaseError> {
        let start = Instant::now();
        let options = FindOptions::builder().sort(doc! { "index": -1 }).limit(1).build();

        let mut cursor = self.get_snos_batch_collection().find(doc! {}, options).await?;
        let batch = cursor.try_next().await?;

        debug!(has_batch = batch.is_some(), category = "db_call", "Retrieved latest SNOS batch");

        let attributes = [KeyValue::new("db_operation_name", "get_latest_snos_batch")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);

        Ok(batch)
    }

    async fn get_snos_batches_by_indices(&self, indexes: Vec<u64>) -> Result<Vec<SnosBatch>, DatabaseError> {
        let start = Instant::now();
        let filter = doc! {
            "index": {
                "$in": indexes.iter().map(|id| bson::to_bson(id).unwrap_or(Bson::Null)).collect::<Vec<Bson>>()
            }
        };

        let batches: Vec<SnosBatch> = self.get_snos_batch_collection().find(filter, None).await?.try_collect().await?;
        debug!(batch_count = batches.len(), category = "db_call", "Retrieved SNOS batches by indices");
        let attributes = [KeyValue::new("db_operation_name", "get_snos_batches_by_indices")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
        Ok(batches)
    }

    async fn update_snos_batch_status_by_index(
        &self,
        index: u64,
        status: SnosBatchStatus,
    ) -> Result<SnosBatch, DatabaseError> {
        let start = Instant::now();
        let filter = doc! {
            "index": index as i64
        };

        let mut updates_doc = Document::new();
        updates_doc.insert("status", Bson::String(format!("{:?}", status)));
        updates_doc.insert("updated_at", Bson::DateTime(Utc::now().round_subsecs(0).into()));

        let update = doc! { "$set": updates_doc };

        let options = FindOneAndUpdateOptions::builder().upsert(false).return_document(ReturnDocument::After).build();
        self.update_snos_batch(filter, update, options, start, index).await
    }

    async fn get_aggregator_batches_by_indexes(
        &self,
        indexes: Vec<u64>,
    ) -> Result<Vec<AggregatorBatch>, DatabaseError> {
        let start = Instant::now();
        let filter = doc! {
            "index": {
                "$in": indexes.iter().map(|index| bson::to_bson(index).unwrap_or(Bson::Null)).collect::<Vec<Bson>>()
            }
        };

        let batches: Vec<AggregatorBatch> =
            self.get_aggregator_batch_collection().find(filter, None).await?.try_collect().await?;
        debug!(batch_count = batches.len(), category = "db_call", "Retrieved aggregator batches by indexes");
        let attributes = [KeyValue::new("db_operation_name", "get_aggregator_batches_by_indexes")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
        Ok(batches)
    }

    /// Update an aggregator batch status by its index
    async fn update_aggregator_batch_status_by_index(
        &self,
        index: u64,
        status: AggregatorBatchStatus,
    ) -> Result<AggregatorBatch, DatabaseError> {
        let start = Instant::now();
        let filter = doc! {
            "index": index as i64
        };

        let mut updates_doc = Document::new();
        updates_doc.insert("status", Bson::String(format!("{:?}", status)));
        updates_doc.insert("updated_at", Bson::DateTime(Utc::now().round_subsecs(0).into()));

        let update = doc! { "$set": updates_doc };

        let options = FindOneAndUpdateOptions::builder().upsert(false).return_document(ReturnDocument::After).build();
        self.update_aggregator_batch(filter, update, options, start, index).await
    }

    /// Updates or create a new aggregator batch
    ///
    /// NOTE: In both cases it'll combine the info in both batch and update arguments
    async fn update_or_create_aggregator_batch(
        &self,
        batch: &AggregatorBatch,
        update: &AggregatorBatchUpdates,
    ) -> Result<AggregatorBatch, DatabaseError> {
        let start = Instant::now();
        let filter = doc! {
            "_id": batch.id,
        };
        let options = FindOneAndUpdateOptions::builder().upsert(true).return_document(ReturnDocument::After).build();

        // Document to store non null values
        let mut non_null_updates = Document::new();

        // remove null values from the batch
        batch.to_document()?.iter().for_each(|(k, v)| {
            if v != &Bson::Null {
                non_null_updates.insert(k, v);
            }
        });
        // remove null values from the update
        update.to_document()?.iter_mut().for_each(|(k, v)| {
            if v != &Bson::Null {
                non_null_updates.insert(k, v);
            }
        });

        // throw an error if there's no field to be updated
        if non_null_updates.is_empty() {
            return Err(DatabaseError::NoUpdateFound("No field to be updated, likely a false call".to_string()));
        }

        // Add additional fields that are always updated
        if let Some(end_block) = update.end_block {
            non_null_updates.insert("num_blocks", Bson::Int64(end_block as i64 - batch.start_block as i64 + 1));
        }
        non_null_updates.insert("updated_at", Bson::DateTime(Utc::now().round_subsecs(0).into()));

        let update = doc! {
            "$set": non_null_updates
        };

        self.update_aggregator_batch(filter, update, options, start, batch.index).await
    }

    async fn update_aggregator_batch(
        &self,
        filter: Document,
        update: Document,
        options: FindOneAndUpdateOptions,
        start: Instant,
        index: u64,
    ) -> Result<AggregatorBatch, DatabaseError> {
        // Find a batch and update it
        let result = self.get_aggregator_batch_collection().find_one_and_update(filter, update, options).await?;
        match result {
            Some(updated_batch) => {
                // Update done
                let attributes = [KeyValue::new("db_operation_name", "update_aggregator_batch")];
                let duration = start.elapsed();
                ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
                Ok(updated_batch)
            }
            None => {
                // Not found
                tracing::error!(index = %index, category = "db_call", "Failed to update batch");
                Err(DatabaseError::UpdateFailed(format!("Failed to update batch. Identifier - {}, ", index)))
            }
        }
    }

    async fn update_snos_batch(
        &self,
        filter: Document,
        update: Document,
        options: FindOneAndUpdateOptions,
        start: Instant,
        index: u64,
    ) -> Result<SnosBatch, DatabaseError> {
        // Find a batch and update it
        let result = self.get_snos_batch_collection().find_one_and_update(filter, update, options).await?;
        match result {
            Some(updated_batch) => {
                // Update done
                let attributes = [KeyValue::new("db_operation_name", "update_snos_batch")];
                let duration = start.elapsed();
                ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
                Ok(updated_batch)
            }
            None => {
                // Not found
                error!(index = %index, "Failed to update batch");
                Err(DatabaseError::UpdateFailed(format!("Failed to update batch. Identifier - {}, ", index)))
            }
        }
    }

    async fn create_aggregator_batch(&self, batch: AggregatorBatch) -> Result<AggregatorBatch, DatabaseError> {
        let start = Instant::now();

        match self
            .get_aggregator_batch_collection()
            .insert_one(batch.clone(), InsertOneOptions::builder().build())
            .await
        {
            Ok(_) => {
                let duration = start.elapsed();
                debug!(duration = %duration.as_millis(), "Batch created in MongoDB successfully");

                let attributes = [KeyValue::new("db_operation_name", "create_batch")];
                ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
                Ok(batch)
            }
            Err(err) => {
                error!(batch_id = %batch.id, "Failed to insert batch");
                Err(DatabaseError::InsertFailed(format!(
                    "Failed to insert batch {} with id {}: {}",
                    batch.index, batch.id, err
                )))
            }
        }
    }

    async fn create_snos_batch(&self, batch: SnosBatch) -> Result<SnosBatch, DatabaseError> {
        let start = Instant::now();
        let collection: Collection<SnosBatch> = self.get_snos_batch_collection();
        match collection.insert_one(batch.clone(), InsertOneOptions::builder().build()).await {
            Ok(_) => {
                let duration = start.elapsed();
                tracing::debug!(duration = %duration.as_millis(), "Batch created in MongoDB successfully");
                Ok(batch)
            }
            Err(err) => {
                tracing::error!(batch_id = %batch.id, category = "db_call", "Failed to insert batch");
                Err(DatabaseError::InsertFailed(format!(
                    "Failed to insert batch {} with id {}: {}",
                    batch.index, batch.id, err
                )))
            }
        }
    }

    async fn update_or_create_snos_batch(
        &self,
        batch: &SnosBatch,
        update: &SnosBatchUpdates,
    ) -> Result<SnosBatch, DatabaseError> {
        let start = Instant::now();
        let filter = doc! {
            "_id": batch.id,
        };
        let options = FindOneAndUpdateOptions::builder().upsert(true).return_document(ReturnDocument::After).build();

        let updates = batch.to_document()?;

        // remove null values from the updates
        let mut non_null_updates = Document::new();
        updates.iter().for_each(|(k, v)| {
            if v != &Bson::Null {
                non_null_updates.insert(k, v);
            }
        });
        update.to_document()?.iter_mut().for_each(|(k, v)| {
            if v != &Bson::Null {
                non_null_updates.insert(k, v);
            }
        });

        // throw an error if there's no field to be updated
        if non_null_updates.is_empty() {
            return Err(DatabaseError::NoUpdateFound("No field to be updated, likely a false call".to_string()));
        }

        // Add additional fields that are always updated
        if let Some(end_block) = update.end_block {
            non_null_updates.insert("num_blocks", Bson::Int64(end_block as i64 - batch.start_block as i64 + 1));
        }
        non_null_updates.insert("updated_at", Bson::DateTime(Utc::now().round_subsecs(0).into()));

        let update = doc! {
            "$set": non_null_updates
        };

        let collection: Collection<SnosBatch> = self.get_snos_batch_collection();
        let result = collection.find_one_and_update(filter, update, options).await?;
        match result {
            Some(updated_batch) => {
                let attributes = [KeyValue::new("db_operation_name", "update_or_create_snos_batch")];
                let duration = start.elapsed();
                ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
                Ok(updated_batch)
            }
            None => {
                tracing::error!(batch_id = %batch.id, category = "db_call", "Failed to update snos batch");
                Err(DatabaseError::UpdateFailed(format!("Failed to update snos batch. Identifier - {}, ", batch.id)))
            }
        }
    }

    /// Get the aggregator batch that contains a specific block number
    async fn get_aggregator_batch_for_block(
        &self,
        block_number: u64,
    ) -> Result<Option<AggregatorBatch>, DatabaseError> {
        let start = Instant::now();
        let filter = doc! {
            "start_block": { "$lte": block_number as i64 },
            "end_block": { "$gte": block_number as i64 }
        };

        let batch = self.get_aggregator_batch_collection().find_one(filter, None).await?;

        debug!("Retrieved aggregator batch by block number");
        let attributes = [KeyValue::new("db_operation_name", "get_aggregator_batch_for_block")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);

        Ok(batch)
    }

    async fn get_start_snos_batch_for_aggregator(
        &self,
        aggregator_index: u64,
    ) -> Result<Option<SnosBatch>, DatabaseError> {
        let start = Instant::now();

        // Construct the aggregation pipeline
        let pipeline = vec![
            // Stage 1: Match by type + status
            doc! {
                "$match": {
                    "aggregator_batch_index": aggregator_index as i64
                }
            },
            // Stage 2: Sort by index ascending
            doc! {
                "$sort": {
                    "index": 1
                }
            },
            // Stage 3: Take only the top document
            doc! { "$limit": 1 },
        ];

        debug!("Fetching first SNOS batch in an Aggregator batch");

        let collection: Collection<SnosBatch> = self.get_snos_batch_collection();

        // Execute pipeline
        let results = self.execute_pipeline::<SnosBatch, SnosBatch>(collection, pipeline, None).await?;

        let attributes = [KeyValue::new("db_operation_name", "get_start_snos_batch_for_aggregator")];

        let result = vec_to_single_result(results, "get_start_snos_batch_for_aggregator")?;

        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
        Ok(result)
    }

    /// Get aggregator batches filtered by status
    async fn get_aggregator_batches_by_status(
        &self,
        status: AggregatorBatchStatus,
        limit: Option<i64>,
    ) -> Result<Vec<AggregatorBatch>, DatabaseError> {
        let start = Instant::now();
        let filter = doc! {
            "status": status.to_string(),
        };
        let find_options_builder = FindOptions::builder().sort(doc! {"index": 1});
        let find_options = limit.map(|val| find_options_builder.limit(Some(val)).build());

        let batches: Vec<AggregatorBatch> =
            self.get_aggregator_batch_collection().find(filter, find_options).await?.try_collect().await?;

        debug!("Retrieved aggregator batches by status");
        let attributes = [KeyValue::new("db_operation_name", "get_aggregator_batches_by_status")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);

        Ok(batches)
    }

    /// Get SNOS batches filtered by status
    async fn get_snos_batches_by_status(
        &self,
        status: SnosBatchStatus,
        limit: Option<i64>,
    ) -> Result<Vec<SnosBatch>, DatabaseError> {
        let start = Instant::now();
        let filter = doc! {
            "status": status.to_string(),
        };
        let find_options_builder = FindOptions::builder().sort(doc! {"index": 1});
        let find_options = limit.map(|val| find_options_builder.limit(Some(val)).build());

        let batches: Vec<SnosBatch> =
            self.get_snos_batch_collection().find(filter, find_options).await?.try_collect().await?;

        tracing::debug!(
            status = %status,
            batch_count = batches.len(),
            category = "db_call",
            "Retrieved SNOS batches by status"
        );
        let attributes = [KeyValue::new("db_operation_name", "get_snos_batches_by_status")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);

        Ok(batches)
    }

    async fn get_snos_batches_without_jobs(
        &self,
        snos_batch_status: SnosBatchStatus,
    ) -> Result<Vec<SnosBatch>, DatabaseError> {
        let start = Instant::now();

        // Convert enums to Bson strings for MongoDB queries
        let snos_batch_status_str = snos_batch_status.to_string();
        let snos_job_type_bson = Bson::String(format!("{:?}", JobType::SnosRun));

        // Construct the aggregation pipeline
        let pipeline = vec![
            // Stage 1: Match SNOS batches with the specified status
            doc! {
                "$match": {
                    "status": snos_batch_status_str
                }
            },
            // Stage 2: Lookup to find corresponding SNOS jobs
            // We look for jobs where internal_id matches the index (as string)
            doc! {
                "$lookup": {
                    "from": JOBS_COLLECTION,
                    "let": { "index": { "$toString": "$index" } },
                    "pipeline": [
                        {
                            "$match": {
                                "$expr": {
                                    "$and": [
                                        { "$eq": ["$job_type", snos_job_type_bson] },
                                        { "$eq": ["$internal_id", "$$index"] }
                                    ]
                                }
                            }
                        }
                    ],
                    "as": "corresponding_jobs"
                }
            },
            // Stage 3: Filter to get only SNOS batches that DON'T have corresponding jobs
            doc! {
                "$match": {
                    "corresponding_jobs": { "$eq": [] }
                }
            },
            // Stage 4: Sort by snos_batch_id for consistent ordering
            doc! {
                "$sort": {
                    "index": 1
                }
            },
        ];

        tracing::debug!(
            snos_batch_status = %snos_batch_status,
            category = "db_call",
            "Fetching SNOS batches without corresponding jobs"
        );

        let collection: Collection<SnosBatch> = self.get_snos_batch_collection();
        let result = self.execute_pipeline::<SnosBatch, SnosBatch>(collection, pipeline, None).await?;

        tracing::debug!(
            batch_count = result.len(),
            category = "db_call",
            "Retrieved SNOS batches without corresponding jobs"
        );

        let attributes = [KeyValue::new("db_operation_name", "get_snos_batches_without_jobs")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);

        Ok(result)
    }

    async fn get_jobs_between_internal_ids(
        &self,
        job_type: JobType,
        status: JobStatus,
        gte: u64,
        lte: u64,
    ) -> Result<Vec<JobItem>, DatabaseError> {
        let start = Instant::now();
        let filter = doc! {
            "job_type": bson::to_bson(&job_type)?,
            "status": bson::to_bson(&status)?,
            "$expr": {
                "$and": [
                    { "$gte": [{ "$toInt": "$internal_id" }, gte as i64 ] },
                    { "$lte": [{ "$toInt": "$internal_id" }, lte as i64 ] }
                ]
            }
        };

        let find_options = FindOptions::builder().sort(doc! { "internal_id": 1 }).build();

        let jobs: Vec<JobItem> = self.get_job_collection().find(filter, find_options).await?.try_collect().await?;

        debug!(
            job_type = ?job_type,
            gte = gte,
            lte = lte,
            job_count = jobs.len(),
            "Fetched jobs between internal IDs"
        );

        let attributes = [KeyValue::new("db_operation_name", "get_jobs_between_internal_ids")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);

        Ok(jobs)
    }

    async fn get_jobs_by_type_and_statuses(
        &self,
        job_type: &JobType,
        job_statuses: Vec<JobStatus>,
    ) -> Result<Vec<JobItem>, DatabaseError> {
        let start = Instant::now();
        let filter = doc! {
            "job_type": bson::to_bson(job_type)?,
            "status": {
                "$in": job_statuses.iter().map(|status| bson::to_bson(status).unwrap_or(Bson::Null)).collect::<Vec<Bson>>()
            }
        };

        let find_options = FindOptions::builder().sort(doc! { "internal_id": -1 }).build();

        let jobs: Vec<JobItem> = self.get_job_collection().find(filter, find_options).await?.try_collect().await?;

        debug!(job_count = jobs.len(), "Retrieved jobs by type and statuses");

        let attributes = [KeyValue::new("db_operation_name", "get_jobs_by_type_and_statuses")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);

        Ok(jobs)
    }

    async fn get_jobs_by_block_number(&self, block_number: u64) -> Result<Vec<JobItem>, DatabaseError> {
        let start = Instant::now();
        let block_number_i64 = block_number as i64; // MongoDB typically handles numbers as i32 or i64

        // Query for jobs where metadata.specific.block_number matches
        let query_all = doc! {
            "metadata.specific.block_number": block_number_i64,
            "job_type": {
                "$in": [
                    bson::to_bson(&JobType::ProofCreation)?,
                    bson::to_bson(&JobType::ProofRegistration)?,
                    bson::to_bson(&JobType::DataSubmission)?,
                ]
            }
        };

        // Query for StateTransition jobs where metadata.specific.blocks_to_settle contains the block_number
        let query_state_transition = doc! {
            "job_type": bson::to_bson(&JobType::StateTransition)?,
            "metadata.specific.context.to_settle": { "$elemMatch": { "$eq": block_number_i64 } }
        };

        // Query for SnosRun and Aggregator jobs
        let query_snos_and_aggregator = doc! {
           "job_type": {
                "$in": [
                    bson::to_bson(&JobType::SnosRun)?,
                    bson::to_bson(&JobType::Aggregator)?,
                ]
            },
            "metadata.specific.start_block": { "$lte": block_number_i64 },
            "metadata.specific.end_block": { "$gte": block_number_i64 }
        };

        let mut results: Vec<JobItem> = Vec::new();

        let job_collection = self.get_job_collection();

        // Execute query for all jobs
        let cursor_all = job_collection.find(query_all, None).await?;
        results.extend(cursor_all.try_collect::<Vec<JobItem>>().await?);

        // Execute query for state transition jobs
        let cursor_state_transition = job_collection.find(query_state_transition, None).await?;
        results.extend(cursor_state_transition.try_collect::<Vec<JobItem>>().await?);

        // Execute query for snos and aggregator jobs
        let cursor_snos_and_aggregator = job_collection.find(query_snos_and_aggregator, None).await?;
        results.extend(cursor_snos_and_aggregator.try_collect::<Vec<JobItem>>().await?);

        debug!(count = results.len(), "Fetched jobs by block number");
        let attributes = [KeyValue::new("db_operation_name", "get_jobs_by_block_number")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);

        Ok(results)
    }

    async fn get_orphaned_jobs(&self, job_type: &JobType, timeout_seconds: u64) -> Result<Vec<JobItem>, DatabaseError> {
        let start = Instant::now();

        // Calculate the cutoff time (current time - timeout)
        let cutoff_time = Utc::now() - chrono::Duration::seconds(timeout_seconds as i64);

        // Query for jobs of the specific type in LockedForProcessing status with
        // process_started_at older than cutoff
        let filter = doc! {
            "job_type": bson::to_bson(job_type)?,
            "status": bson::to_bson(&JobStatus::LockedForProcessing)?,
            "metadata.common.process_started_at": {
                "$lt": cutoff_time.timestamp()
            }
        };

        let jobs: Vec<JobItem> = self.get_job_collection().find(filter, None).await?.try_collect().await?;

        debug!(
            cutoff_time = %cutoff_time,
            orphaned_count = jobs.len(),
            "Found orphaned jobs in LockedForProcessing status"
        );

        let attributes = [KeyValue::new("db_operation_name", "get_orphaned_jobs")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);

        Ok(jobs)
    }

    async fn get_jobs_by_status(&self, status: JobStatus) -> Result<Vec<JobItem>, DatabaseError> {
        let start = Instant::now();

        let filter = doc! {
            "status": bson::to_bson(&status)?,
        };

        let jobs: Vec<JobItem> = self.get_job_collection().find(filter, None).await?.try_collect().await?;

        debug!(job_count = jobs.len(), "Fetched jobs by status");

        let attributes = [KeyValue::new("db_operation_name", "get_jobs_by_status")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);

        Ok(jobs)
    }

    // ================================================================================
    // Batch Relationship Management Methods
    // ================================================================================

    /// Get all SNOS batches belonging to a specific aggregator batch
    async fn get_snos_batches_by_aggregator_index(
        &self,
        aggregator_index: u64,
    ) -> Result<Vec<SnosBatch>, DatabaseError> {
        let start = Instant::now();

        let find_options = FindOptions::builder().sort(doc! { "index": 1 }).build();

        let filter = doc! {
            "aggregator_batch_index": aggregator_index as i64
        };

        let batches: Vec<SnosBatch> =
            self.get_snos_batch_collection().find(filter, find_options).await?.try_collect().await?;

        tracing::debug!(
            aggregator_index = aggregator_index,
            snos_batch_count = batches.len(),
            category = "db_call",
            "Retrieved SNOS batches by aggregator index"
        );

        let attributes = [KeyValue::new("db_operation_name", "get_snos_batches_by_aggregator_index")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
        Ok(batches)
    }

    /// Get open SNOS batches for a specific aggregator batch
    async fn get_open_snos_batches_by_aggregator_index(
        &self,
        aggregator_index: u64,
    ) -> Result<Vec<SnosBatch>, DatabaseError> {
        let start = Instant::now();
        let filter = doc! {
            "aggregator_batch_index": aggregator_index as i64,
            "status": "Open"
        };

        let batches: Vec<SnosBatch> = self.get_snos_batch_collection().find(filter, None).await?.try_collect().await?;

        tracing::debug!(
            aggregator_index = aggregator_index,
            open_snos_batch_count = batches.len(),
            category = "db_call",
            "Retrieved open SNOS batches by aggregator index"
        );

        let attributes = [KeyValue::new("db_operation_name", "get_open_snos_batches_by_aggregator_index")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
        Ok(batches)
    }

    /// Get the next available SNOS batch ID
    async fn get_next_snos_batch_id(&self) -> Result<u64, DatabaseError> {
        let start = Instant::now();
        let options = FindOptions::builder().sort(doc! { "index": -1 }).limit(1).build();

        let mut cursor = self.get_snos_batch_collection().find(doc! {}, options).await?;
        let latest_batch = cursor.try_next().await?;

        let next_id = latest_batch.map_or(1, |batch| batch.index + 1);

        tracing::debug!(next_snos_batch_id = next_id, category = "db_call", "Generated next SNOS batch ID");

        let attributes = [KeyValue::new("db_operation_name", "get_next_snos_batch_id")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
        Ok(next_id)
    }

    /// Close all SNOS batches for a specific aggregator batch
    async fn close_all_snos_batches_for_aggregator(
        &self,
        aggregator_index: u64,
    ) -> Result<Vec<SnosBatch>, DatabaseError> {
        let start = Instant::now();
        let filter = doc! {
            "aggregator_batch_index": aggregator_index as i64,
            "status": { "$ne": "Closed" }
        };

        let update = doc! {
            "$set": {
                "status": "Closed",
                "updated_at": Bson::DateTime(Utc::now().round_subsecs(0).into())
            }
        };

        // Update all matching documents
        let update_result = self.get_snos_batch_collection().update_many(filter.clone(), update, None).await?;

        tracing::debug!(
            aggregator_index = aggregator_index,
            closed_snos_batches = update_result.modified_count,
            category = "db_call",
            "Closed SNOS batches for aggregator"
        );

        // Return the updated batches by querying for closed batches
        let updated_filter = doc! {
            "aggregator_batch_index": aggregator_index as i64,
            "status": "Closed"
        };

        let updated_batches: Vec<SnosBatch> =
            self.get_snos_batch_collection().find(updated_filter, None).await?.try_collect().await?;

        let attributes = [KeyValue::new("db_operation_name", "close_all_snos_batches_for_aggregator")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
        Ok(updated_batches)
    }

    async fn health_check(&self) -> Result<(), DatabaseError> {
        let start = Instant::now();

        // Perform a simple ping operation to verify connectivity
        // This is a lightweight operation that checks if the database is accessible
        self.database.run_command(doc! { "ping": 1 }, None).await?;

        let attributes = [KeyValue::new("db_operation_name", "health_check")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
        Ok(())
    }
}

// Generic utility function to convert Vec<T> to Option<T>
fn vec_to_single_result<T>(results: Vec<T>, operation_name: &str) -> Result<Option<T>, DatabaseError> {
    match results.len() {
        0 => Ok(None),
        1 => Ok(results.into_iter().next()),
        n => {
            error!("Expected at most 1 result, got {} for operation: {}", n, operation_name);
            Err(DatabaseError::FailedToSerializeDocument(format!(
                "Expected at most 1 result, got {} for operation: {}",
                n, operation_name
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::doc;
    use mongodb::options::ClientOptions;
    use serde::{Deserialize, Serialize};
    use std::env;

    #[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
    struct TestDoc {
        _id: i32,
        name: String,
    }

    async fn get_test_handles() -> (Client, Database, Collection<TestDoc>) {
        let uri = env::var("MONGODB_URI").unwrap_or_else(|_| "mongodb://localhost:27017".to_string());
        let client_options = ClientOptions::parse(&uri).await.unwrap();
        let client = Client::with_options(client_options).unwrap();
        let db = client.database("test_db");
        let collection = db.collection::<TestDoc>("test_collection");
        (client, db, collection)
    }

    #[tokio::test]
    async fn test_find_one_insert_and_delete() {
        let (client, db, collection) = get_test_handles().await;
        // Clean up before test
        let _ = collection.delete_many(doc! {}, None).await;
        let test_doc = TestDoc { _id: 1, name: "Alice".to_string() };
        collection.insert_one(&test_doc, None).await.unwrap();

        let client = MongoDbClient { client, database: Arc::new(db) };

        // find_one
        let found = client.find_one(collection.clone(), doc! {"_id": 1}).await.unwrap();
        assert_eq!(found, Some(test_doc.clone()));

        // update_one
        let update = doc! { "$set": { "name": "Bob" } };
        let update_result = client.update_one(collection.clone(), doc! {"_id": 1}, update, None).await.unwrap();
        assert_eq!(update_result.matched_count, 1);
        assert_eq!(update_result.modified_count, 1);

        // find (should return updated doc)
        let found_docs = client.find(collection.clone(), doc! {"_id": 1}, None, None, None, None).await.unwrap();
        assert_eq!(found_docs.len(), 1);
        assert_eq!(found_docs[0].name, "Bob");

        // delete_one
        let delete_result = client.delete_one(collection.clone(), doc! {"_id": 1}).await.unwrap();
        assert_eq!(delete_result.deleted_count, 1);

        // find_one (should be None)
        let found = client.find_one(collection.clone(), doc! {"_id": 1}).await.unwrap();
        assert_eq!(found, None);
    }
}
