use super::error::DatabaseError;
use crate::core::client::database::constant::{BATCHES_COLLECTION, JOBS_COLLECTION};
use crate::core::client::database::DatabaseClient;
use crate::core::client::lock::constant::LOCKS_COLLECTION;
use crate::types::batch::{Batch, BatchStatus, BatchUpdates};
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

    fn get_batch_collection(&self) -> Collection<Batch> {
        self.database.collection(BATCHES_COLLECTION)
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

    /// Function to get missing block numbers for jobs within a specified range.
    ///
    /// `job_type`: Type of job to check for missing blocks.
    /// `lower_cap`: The minimum block number (inclusive).
    /// `upper_cap`: The maximum block number (exclusive, following MongoDB $range behavior).
    ///
    /// Returns a vector of missing block numbers within the specified range.
    ///
    /// Example use case:
    /// Getting missing SNOS jobs between blocks 2000 and 70,000
    ///
    /// # Arguments
    /// job_type: SnosRun
    /// lower_cap: 2000
    /// upper_cap: 70000
    async fn get_missing_block_numbers_by_type_and_caps(
        &self,
        job_type: JobType,
        lower_cap: u64,
        upper_cap: u64,
        limit: Option<i64>,
    ) -> Result<Vec<u64>, DatabaseError> {
        let start = Instant::now();
        // Converting params to Bson
        let job_type_bson = bson::to_bson(&job_type)?;

        // NOTE: This implementation is limited by mongodb's ability to not support u64.
        // i.e., it will fail if upper_limit / lower_limit exceeds u32::MAX.

        let lower_limit = u32::try_from(lower_cap).map_err(|e| {
            error!(error = %e, "Deserialization error");
            DatabaseError::FailedToSerializeDocument(format!("Failed to deserialize: {}", e))
        })?;
        let upper_limit = u32::try_from(upper_cap.saturating_add(1)).map_err(|e| {
            error!(error = %e, "Deserialization error");
            DatabaseError::FailedToSerializeDocument(format!("Failed to deserialize: {}", e))
        })?;

        // Constructing the aggregation pipeline
        let mut pipeline = vec![
            doc! {
                "$facet": {
                    "existing_data": [
                        doc! {
                            "$match": {
                                "job_type": job_type_bson,
                                "metadata.specific.block_number": {
                                    "$gte": lower_limit,
                                    "$lt": upper_limit
                                }
                            }
                        },
                        doc! {
                            "$group": {
                                "_id": null,
                                "existing_blocks": {
                                    "$addToSet": "$metadata.specific.block_number"
                                }
                            }
                        }
                    ]
                }
            },
            doc! {
                "$project": {
                    "existing_blocks": {
                        "$ifNull": [
                            { "$arrayElemAt": ["$existing_data.existing_blocks", 0] },
                            []
                        ]
                    }
                }
            },
            doc! {
                "$addFields": {
                    "complete_range": {
                        "$range": [lower_limit, upper_limit]
                    }
                }
            },
            doc! {
                "$project": {
                    "missing_blocks": {
                        "$setDifference": [
                            "$complete_range",
                            "$existing_blocks"
                        ]
                    }
                }
            },
        ];

        if let Some(limit_value) = limit {
            pipeline.push(doc! {
                "$project": {
                    "missing_blocks": {
                        "$slice": ["$missing_blocks", limit_value]
                    }
                }
            });
        }

        debug!("Fetching missing jobs by type and caps");

        let collection: Collection<JobItem> = self.get_job_collection();

        // Execute the pipeline and extract block numbers
        let missing_blocks_response =
            self.execute_pipeline::<JobItem, MissingBlocksResponse>(collection, pipeline, None).await?;

        debug!(job_count = missing_blocks_response.len(), "Retrieved missing jobs");

        // Handle the case where we might not get any results
        let block_numbers = if missing_blocks_response.is_empty() {
            Vec::new()
        } else {
            let mut block_numbers = missing_blocks_response[0].missing_blocks.clone();
            block_numbers.sort();
            block_numbers
        };

        let attributes = [KeyValue::new("db_operation_name", "get_missing_block_numbers_by_type_and_caps")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);

        Ok(block_numbers)
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

    async fn get_latest_batch(&self) -> Result<Option<Batch>, DatabaseError> {
        let start = Instant::now();
        let pipeline = vec![
            doc! {
                "$sort": {
                    "index": -1
                }
            },
            doc! {
                "$limit": 1
            },
        ];

        let mut cursor = self.get_batch_collection().aggregate(pipeline, None).await?;

        match cursor.try_next().await? {
            Some(doc) => {
                // Try to deserialize and log any errors
                match bson::from_document::<Batch>(doc.clone()) {
                    Ok(batch) => {
                        let attributes = [KeyValue::new("db_operation_name", "get_latest_batch")];
                        let duration = start.elapsed();
                        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
                        Ok(Some(batch))
                    }
                    Err(e) => {
                        error!(
                            error = %e,
                            document = ?doc,
                            "Failed to deserialize document into Batch"
                        );
                        Err(DatabaseError::FailedToSerializeDocument(format!("Failed to deserialize document: {}", e)))
                    }
                }
            }
            None => Ok(None),
        }
    }

    async fn get_batches_by_indexes(&self, indexes: Vec<u64>) -> Result<Vec<Batch>, DatabaseError> {
        let start = Instant::now();
        let filter = doc! {
            "index": {
                "$in": indexes.iter().map(|status| bson::to_bson(status).unwrap_or(Bson::Null)).collect::<Vec<Bson>>()
            }
        };

        let jobs: Vec<Batch> = self.get_batch_collection().find(filter, None).await?.try_collect().await?;
        debug!(job_count = jobs.len(), "Retrieved batch by indexes");
        let attributes = [KeyValue::new("db_operation_name", "get_batches_by_indexes")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
        Ok(jobs)
    }

    /// Update a batch by its index
    async fn update_batch_status_by_index(&self, index: u64, status: BatchStatus) -> Result<Batch, DatabaseError> {
        let start = Instant::now();
        let filter = doc! { "index": index as i64 };

        let mut updates_doc = Document::new();
        updates_doc.insert("status", Bson::String(format!("{:?}", status)));
        updates_doc.insert("updated_at", Bson::DateTime(Utc::now().round_subsecs(0).into()));

        let update = doc! { "$set": updates_doc };

        let options = FindOneAndUpdateOptions::builder().upsert(false).return_document(ReturnDocument::After).build();
        self.update_batch(filter, update, options, start, index).await
    }

    async fn update_or_create_batch(&self, batch: &Batch, update: &BatchUpdates) -> Result<Batch, DatabaseError> {
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

        self.update_batch(filter, update, options, start, batch.index).await
    }

    async fn update_batch(
        &self,
        filter: Document,
        update: Document,
        options: FindOneAndUpdateOptions,
        start: Instant,
        index: u64,
    ) -> Result<Batch, DatabaseError> {
        // Find a batch and update it
        let result = self.get_batch_collection().find_one_and_update(filter, update, options).await?;
        match result {
            Some(updated_batch) => {
                // Update done
                let attributes = [KeyValue::new("db_operation_name", "update_batch")];
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

    async fn create_batch(&self, batch: Batch) -> Result<Batch, DatabaseError> {
        let start = Instant::now();

        match self.get_batch_collection().insert_one(batch.clone(), InsertOneOptions::builder().build()).await {
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

    /// get_batch_for_block - Returns the batch for a given block number
    async fn get_batch_for_block(&self, block_number: u64) -> Result<Option<Batch>, DatabaseError> {
        let start = Instant::now();
        let filter = doc! {
            "start_block": { "$lte": block_number as i64 },
            "end_block": { "$gte": block_number as i64 }
        };

        let batch = self.get_batch_collection().find_one(filter, None).await?;

        debug!("Retrieved batch by block number");
        let attributes = [KeyValue::new("db_operation_name", "get_batch_for_block")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);

        Ok(batch)
    }

    /// get_batches_by_status - Returns a vector of Batch for which the status is the given status
    async fn get_batches_by_status(
        &self,
        status: BatchStatus,
        limit: Option<i64>,
    ) -> Result<Vec<Batch>, DatabaseError> {
        let start = Instant::now();
        let filter = doc! {
            "status": status.to_string(),
        };
        let find_options_builder = FindOptions::builder().sort(doc! {"index": 1});
        let find_options = limit.map(|val| find_options_builder.limit(Some(val)).build());

        let batches = self.get_batch_collection().find(filter, find_options).await?.try_collect().await?;

        debug!("Retrieved batches by statuses");
        let attributes = [KeyValue::new("db_operation_name", "get_all_batches_by_status")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);

        Ok(batches)
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
        let query1 = doc! {
            "metadata.specific.block_number": block_number_i64,
            "job_type": {
                "$in": [
                    bson::to_bson(&JobType::SnosRun)?,
                    bson::to_bson(&JobType::ProofCreation)?,
                    bson::to_bson(&JobType::ProofRegistration)?,
                    bson::to_bson(&JobType::DataSubmission)?,
                    bson::to_bson(&JobType::Aggregator)?,
                ]
            }
        };

        // Query for StateTransition jobs where metadata.specific.blocks_to_settle contains the block_number
        let query2 = doc! {
            "job_type": bson::to_bson(&JobType::StateTransition)?,
            "metadata.specific.context.to_settle": { "$elemMatch": { "$eq": block_number_i64 } }
        };

        let mut results: Vec<JobItem> = Vec::new();

        let job_collection = self.get_job_collection();

        // Execute first query
        let cursor1 = job_collection.find(query1, None).await?;
        results.extend(cursor1.try_collect::<Vec<JobItem>>().await?);

        // Execute second query
        let cursor2 = job_collection.find(query2, None).await?;
        results.extend(cursor2.try_collect::<Vec<JobItem>>().await?);

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
