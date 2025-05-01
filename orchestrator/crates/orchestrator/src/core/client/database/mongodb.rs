use super::error::DatabaseError;
use crate::core::client::database::DatabaseClient;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::job_updates::JobItemUpdates;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::types::params::database::DatabaseArgs;
use crate::utils::metrics::ORCHESTRATOR_METRICS;
use async_trait::async_trait;
use chrono::{SubsecRound, Utc};
use futures::{StreamExt, TryStreamExt};
use mongodb::bson::{doc, Bson, Document};
use mongodb::options::{FindOneAndUpdateOptions, FindOneOptions, FindOptions, ReturnDocument, UpdateOptions};
use mongodb::{bson, Client, Collection, Database};
use opentelemetry::KeyValue;
use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

pub trait ToDocument {
    fn to_document(&self) -> Result<Document, DatabaseError>;
}

impl<T: Serialize> ToDocument for T {
    fn to_document(&self) -> Result<Document, DatabaseError> {
        let doc = mongodb::bson::to_bson(self)?;

        if let Bson::Document(doc) = doc {
            Ok(doc)
        } else {
            Err(DatabaseError::FailedToSerializeDocument(format!("Failed to serialize document: {}", doc)))
        }
    }
}

/// MongoDB client implementation
pub struct MongoDbClient {
    client: Client,
    database: Arc<Database>,
}

impl MongoDbClient {
    pub async fn create(config: &DatabaseArgs) -> Result<Self, DatabaseError> {
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
        self.database.collection("jobs")
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
    #[tracing::instrument(skip(self), fields(function_type = "db_call"), ret, err)]
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
            tracing::debug!(duration = %duration.as_millis(), "Job created in MongoDB successfully");

            let attributes = [KeyValue::new("db_operation_name", "create_job")];
            ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
            Ok(job)
        } else {
            return Err(DatabaseError::ItemAlreadyExists(format!(
                "Job already exists for internal_id {} and job_type {:?}",
                job.internal_id, job.job_type
            )));
        }
    }

    #[tracing::instrument(skip(self), fields(function_type = "db_call"), ret, err)]
    async fn get_job_by_id(&self, id: Uuid) -> Result<Option<JobItem>, DatabaseError> {
        let start = Instant::now();
        let filter = doc! {
            "id":  id
        };
        tracing::debug!(job_id = %id, category = "db_call", "Fetched job by ID");
        let attributes = [KeyValue::new("db_operation_name", "get_job_by_id")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
        Ok(self.get_job_collection().find_one(filter, None).await?)
    }

    #[tracing::instrument(skip(self), fields(function_type = "db_call"), ret, err)]
    async fn get_job_by_internal_id_and_type(
        &self,
        internal_id: &str,
        job_type: &JobType,
    ) -> Result<Option<JobItem>, DatabaseError> {
        let start = Instant::now();
        let filter = doc! {
            "internal_id": internal_id,
            "job_type": mongodb::bson::to_bson(&job_type)?,
        };
        tracing::debug!(internal_id = %internal_id, job_type = ?job_type, category = "db_call", "Fetched job by internal ID and type");
        let attributes = [KeyValue::new("db_operation_name", "get_job_by_internal_id_and_type")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
        Ok(self.get_job_collection().find_one(filter, None).await?)
    }

    #[tracing::instrument(skip(self), fields(function_type = "db_call"), ret, err)]
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
                tracing::debug!(job_id = %current_job.id, category = "db_call", "Job updated successfully");
                let attributes = [KeyValue::new("db_operation_name", "update_job")];
                let duration = start.elapsed();
                ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
                Ok(job)
            }
            None => {
                tracing::warn!(job_id = %current_job.id, category = "db_call", "Failed to update job. Job version is likely outdated");
                Err(DatabaseError::UpdateFailed(format!("Failed to update job. Identifier - {}, ", current_job.id)))
            }
        }
    }

    #[tracing::instrument(skip(self), fields(function_type = "db_call"), ret, err)]
    async fn get_latest_job_by_type(&self, job_type: JobType) -> Result<Option<JobItem>, DatabaseError> {
        let start = Instant::now();
        let pipeline = vec![
            doc! {
                "$match": {
                    "job_type": mongodb::bson::to_bson(&job_type)?
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

        let mut cursor = self.get_job_collection().aggregate(pipeline, None).await?;

        tracing::debug!(job_type = ?job_type, category = "db_call", "Fetching latest job by type");

        match cursor.try_next().await? {
            Some(doc) => {
                // Try to deserialize and log any errors
                match mongodb::bson::from_document::<JobItem>(doc.clone()) {
                    Ok(job) => {
                        tracing::debug!(deserialized_job = ?job, "Successfully deserialized job");
                        let attributes = [KeyValue::new("db_operation_name", "get_latest_job_by_type")];
                        let duration = start.elapsed();
                        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
                        Ok(Some(job))
                    }
                    Err(e) => {
                        tracing::error!(
                            error = %e,
                            document = ?doc,
                            "Failed to deserialize document into JobItem"
                        );
                        Err(DatabaseError::FailedToSerializeDocument(format!("Failed to deserialize document: {}", e)))
                    }
                }
            }
            None => Ok(None),
        }
    }

    /// function to get jobs that don't have a successor job.
    ///
    /// `job_a_type` : Type of job that we need to get that doesn't have any successor.
    ///
    /// `job_a_status` : Status of job A.
    ///
    /// `job_b_type` : Type of job that we need to have as a successor for Job A.
    ///
    /// `job_b_status` : Status of job B which we want to check with.
    ///
    /// Eg :
    ///
    /// Getting SNOS jobs that do not have a successive proving job initiated yet.
    ///
    /// job_a_type : SnosRun
    ///
    /// job_a_status : Completed
    ///
    /// job_b_type : ProofCreation
    ///
    /// TODO : For now Job B status implementation is pending so we can pass None
    #[tracing::instrument(skip(self), fields(function_type = "db_call"), ret, err)]
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

        // implement job_b_status here in the pipeline

        // Construct the initial pipeline
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
                    "from": "jobs",
                    "let": { "internal_id": "$internal_id" },
                    "pipeline": [
                        {
                            "$match": {
                                "$expr": {
                                    "$and": [
                                        { "$eq": ["$job_type", job_b_type_bson] },
                                        // Conditionally match job_b_status if provided
                                        { "$eq": ["$internal_id", "$$internal_id"] }
                                    ]
                                }
                            }
                        },
                        // TODO : Job B status code :
                        // // Add status matching if job_b_status is provided
                        // if let Some(status) = job_b_status {
                        //     doc! {
                        //         "$match": {
                        //             "$expr": { "$eq": ["$status", status] }
                        //         }
                        //     }
                        // } else {
                        //     doc! {}
                        // }
                    // ].into_iter().filter(|d| !d.is_empty()).collect::<Vec<_>>(),
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
        // TODO : Job B status code :
        // // Conditionally add status matching for job_b_status
        // if let Some(status) = job_b_status {
        //     let job_b_status_bson = Bson::String(format!("{:?}", status));
        //
        //     // Access the "$lookup" stage in the pipeline and modify the "pipeline" array inside it
        //     if let Ok(lookup_stage) = pipeline[1].get_document_mut("pipeline") {
        //         if let Ok(lookup_pipeline) = lookup_stage.get_array_mut(0) {
        //             lookup_pipeline.push(Bson::Document(doc! {
        //             "$match": {
        //                 "$expr": { "$eq": ["$status", job_b_status_bson] }
        //             }
        //         }));
        //         }
        //     }
        // }

        let mut cursor = self.get_job_collection().aggregate(pipeline, None).await?;

        let mut vec_jobs: Vec<JobItem> = Vec::new();

        // Iterate over the cursor and process each document
        while let Some(result) = cursor.next().await {
            match result {
                Ok(document) => match bson::from_bson(Bson::Document(document)) {
                    Ok(job_item) => vec_jobs.push(job_item),
                    Err(e) => tracing::error!(error = %e, category = "db_call", "Failed to deserialize JobItem"),
                },
                Err(e) => tracing::error!(error = %e, category = "db_call", "Error retrieving document"),
            }
        }

        tracing::debug!(job_count = vec_jobs.len(), category = "db_call", "Retrieved jobs without successor");
        let attributes = [KeyValue::new("db_operation_name", "get_jobs_without_successor")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
        Ok(vec_jobs)
    }

    #[tracing::instrument(skip(self), fields(function_type = "db_call"), ret, err)]
    async fn get_latest_job_by_type_and_status(
        &self,
        job_type: JobType,
        job_status: JobStatus,
    ) -> Result<Option<JobItem>, DatabaseError> {
        let start = Instant::now();
        let filter = doc! {
            "job_type": bson::to_bson(&job_type)?,
            "status": bson::to_bson(&job_status)?
        };
        let find_options = FindOneOptions::builder().sort(doc! { "internal_id": -1 }).build();

        tracing::debug!(job_type = ?job_type, job_status = ?job_status, category = "db_call", "Fetched latest job by type and status");
        let attributes = [KeyValue::new("db_operation_name", "get_latest_job_by_type_and_status")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
        Ok(self.get_job_collection().find_one(filter, find_options).await?)
    }

    #[tracing::instrument(skip(self), fields(function_type = "db_call"), ret, err)]
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
        tracing::debug!(job_type = ?job_type, job_status = ?job_status, internal_id = internal_id, category = "db_call", "Fetched jobs after internal ID by job type");
        let attributes = [KeyValue::new("db_operation_name", "get_jobs_after_internal_id_by_job_type")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
        Ok(jobs)
    }

    async fn get_jobs_by_statuses(
        &self,
        status: Vec<JobStatus>,
        limit: Option<i64>,
    ) -> Result<Vec<JobItem>, DatabaseError> {
        let start = Instant::now();
        let filter = doc! {
            "status": {
                // TODO: Check that the conversion leads to valid output!
                "$in": status.iter().map(|status| bson::to_bson(status).unwrap_or(Bson::Null)).collect::<Vec<Bson>>()
            }
        };

        let find_options = limit.map(|val| FindOptions::builder().limit(Some(val)).build());

        let jobs: Vec<JobItem> = self.get_job_collection().find(filter, find_options).await?.try_collect().await?;
        tracing::debug!(job_count = jobs.len(), category = "db_call", "Retrieved jobs by statuses");
        let attributes = [KeyValue::new("db_operation_name", "get_jobs_by_statuses")];
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);
        Ok(jobs)
    }
}
