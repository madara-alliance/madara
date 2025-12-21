use async_trait::async_trait;
use chrono::{SubsecRound, Utc};
use mongodb::bson::{self, doc, Bson, Document};
use mongodb::options::{FindOneAndUpdateOptions, FindOptions, ReturnDocument};
use std::sync::Arc;
use tracing::{debug, warn};
use uuid::Uuid;

use super::r#trait::JobRepository;
use crate::core::client::database::constant::JOBS_COLLECTION;
use crate::core::client::database::error::DatabaseError;
use crate::core::client::database::mongo_client::helpers::ToDocument;
use crate::core::client::database::mongo_client::MongoClient;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::job_updates::JobItemUpdates;
use crate::types::jobs::types::{JobStatus, JobType};

pub struct MongoJobRepository {
    client: Arc<MongoClient>,
}

impl MongoJobRepository {
    pub fn new(client: Arc<MongoClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl JobRepository for MongoJobRepository {
    async fn create_job(&self, job: JobItem) -> Result<JobItem, DatabaseError> {
        let filter = doc! {
            "job_type": bson::to_bson(&job.job_type)?,
            "internal_id": &job.internal_id,
        };

        let inserted = self.client.insert_if_not_exists(JOBS_COLLECTION, filter, job.clone()).await?;

        if inserted {
            debug!(job_id = %job.id, internal_id = %job.internal_id, "Job created");
            Ok(job)
        } else {
            Err(DatabaseError::ItemAlreadyExists(format!("Job already exists: {} {:?}", job.internal_id, job.job_type)))
        }
    }

    async fn get_job_by_id(&self, id: Uuid) -> Result<Option<JobItem>, DatabaseError> {
        self.client.find_one(JOBS_COLLECTION, doc! { "id": id }).await
    }

    async fn get_job_by_internal_id_and_type(
        &self,
        internal_id: &str,
        job_type: &JobType,
    ) -> Result<Option<JobItem>, DatabaseError> {
        let filter = doc! {
            "internal_id": internal_id,
            "job_type": bson::to_bson(job_type)?,
        };
        self.client.find_one(JOBS_COLLECTION, filter).await
    }

    async fn update_job(&self, current_job: &JobItem, update: JobItemUpdates) -> Result<JobItem, DatabaseError> {
        let filter = doc! {
            "id": current_job.id,
            "version": current_job.version,
        };

        let updates = update.to_document()?;

        // Separate $set and $unset operations
        let mut set_doc = Document::new();
        let mut unset_doc = Document::new();

        for (key, value) in updates.iter() {
            if value == &Bson::Null {
                unset_doc.insert(key, "");
            } else {
                set_doc.insert(key, value);
            }
        }

        // Always update version and updated_at
        set_doc.insert("version", current_job.version + 1);
        set_doc.insert("updated_at", Bson::DateTime(Utc::now().round_subsecs(0).into()));

        let mut update_doc = doc! { "$set": set_doc };
        if !unset_doc.is_empty() {
            update_doc.insert("$unset", unset_doc);
        }

        let options = FindOneAndUpdateOptions::builder().return_document(ReturnDocument::After).build();

        self.client.find_one_and_update::<JobItem>(JOBS_COLLECTION, filter, update_doc, options).await?.ok_or_else(
            || {
                warn!(job_id = %current_job.id, version = current_job.version, "Update failed - version mismatch");
                DatabaseError::UpdateFailed(format!("Job {} version mismatch", current_job.id))
            },
        )
    }

    async fn get_latest_job_by_type(&self, job_type: JobType) -> Result<Option<JobItem>, DatabaseError> {
        let pipeline = vec![
            doc! { "$match": { "job_type": bson::to_bson(&job_type)? } },
            doc! { "$addFields": { "numeric_id": { "$toLong": "$internal_id" } } },
            doc! { "$sort": { "numeric_id": -1 } },
            doc! { "$limit": 1 },
            doc! { "$project": { "numeric_id": 0 } },
        ];

        let results: Vec<JobItem> = self.client.aggregate::<JobItem, JobItem>(JOBS_COLLECTION, pipeline).await?;

        Ok(results.into_iter().next())
    }

    async fn get_latest_job_by_type_and_status(
        &self,
        job_type: JobType,
        job_status: JobStatus,
    ) -> Result<Option<JobItem>, DatabaseError> {
        let pipeline = vec![
            doc! {
                "$match": {
                    "job_type": bson::to_bson(&job_type)?,
                    "status": bson::to_bson(&job_status)?,
                }
            },
            doc! { "$sort": { "metadata.specific.block_number": -1 } },
            doc! { "$limit": 1 },
        ];

        let results: Vec<JobItem> = self.client.aggregate::<JobItem, JobItem>(JOBS_COLLECTION, pipeline).await?;

        Ok(results.into_iter().next())
    }

    async fn get_jobs_by_status(&self, status: JobStatus) -> Result<Vec<JobItem>, DatabaseError> {
        let filter = doc! { "status": bson::to_bson(&status)? };
        self.client.find_many(JOBS_COLLECTION, filter, None).await
    }

    async fn get_jobs_by_types_and_statuses(
        &self,
        job_types: Vec<JobType>,
        statuses: Vec<JobStatus>,
        limit: Option<i64>,
    ) -> Result<Vec<JobItem>, DatabaseError> {
        let mut filter = doc! {};

        if !job_types.is_empty() {
            let types: Vec<Bson> = job_types.iter().map(|t| bson::to_bson(t)).collect::<Result<_, _>>()?;
            filter.insert("job_type", doc! { "$in": types });
        }

        if !statuses.is_empty() {
            let statuses: Vec<Bson> = statuses.iter().map(|s| bson::to_bson(s)).collect::<Result<_, _>>()?;
            filter.insert("status", doc! { "$in": statuses });
        }

        let options = limit.map(|l| FindOptions::builder().limit(l).build());
        self.client.find_many(JOBS_COLLECTION, filter, options).await
    }

    async fn get_jobs_by_type_and_statuses(
        &self,
        job_type: &JobType,
        statuses: Vec<JobStatus>,
    ) -> Result<Vec<JobItem>, DatabaseError> {
        let statuses: Vec<Bson> = statuses.iter().map(|s| bson::to_bson(s)).collect::<Result<_, _>>()?;

        let filter = doc! {
            "job_type": bson::to_bson(job_type)?,
            "status": { "$in": statuses },
        };

        let options = FindOptions::builder().sort(doc! { "internal_id": -1 }).build();

        self.client.find_many(JOBS_COLLECTION, filter, Some(options)).await
    }

    async fn get_jobs_after_internal_id_by_job_type(
        &self,
        job_type: JobType,
        job_status: JobStatus,
        internal_id: String,
    ) -> Result<Vec<JobItem>, DatabaseError> {
        let filter = doc! {
            "job_type": bson::to_bson(&job_type)?,
            "status": bson::to_bson(&job_status)?,
            "$expr": {
                "$gt": [
                    { "$toInt": "$internal_id" },
                    { "$toInt": &internal_id }
                ]
            }
        };
        self.client.find_many(JOBS_COLLECTION, filter, None).await
    }

    async fn get_jobs_between_internal_ids(
        &self,
        job_type: JobType,
        status: JobStatus,
        from_id: u64,
        to_id: u64,
    ) -> Result<Vec<JobItem>, DatabaseError> {
        let filter = doc! {
            "job_type": bson::to_bson(&job_type)?,
            "status": bson::to_bson(&status)?,
            "$expr": {
                "$and": [
                    { "$gte": [{ "$toInt": "$internal_id" }, from_id as i64] },
                    { "$lte": [{ "$toInt": "$internal_id" }, to_id as i64] }
                ]
            }
        };

        let options = FindOptions::builder().sort(doc! { "internal_id": 1 }).build();

        self.client.find_many(JOBS_COLLECTION, filter, Some(options)).await
    }

    async fn get_jobs_by_block_number(&self, block_number: u64) -> Result<Vec<JobItem>, DatabaseError> {
        let block = block_number as i64;

        // Query 1: Jobs with direct block_number field
        let filter1 = doc! {
            "metadata.specific.block_number": block,
            "job_type": {
                "$in": [
                    bson::to_bson(&JobType::ProofCreation)?,
                    bson::to_bson(&JobType::ProofRegistration)?,
                    bson::to_bson(&JobType::DataSubmission)?,
                ]
            }
        };

        // Query 2: StateTransition jobs with block in to_settle array
        let filter2 = doc! {
            "job_type": bson::to_bson(&JobType::StateTransition)?,
            "metadata.specific.context.to_settle": { "$elemMatch": { "$eq": block } }
        };

        // Query 3: SNOS/Aggregator jobs with block in range
        let filter3 = doc! {
            "job_type": {
                "$in": [
                    bson::to_bson(&JobType::SnosRun)?,
                    bson::to_bson(&JobType::Aggregator)?,
                ]
            },
            "metadata.specific.start_block": { "$lte": block },
            "metadata.specific.end_block": { "$gte": block }
        };

        let mut results = Vec::new();
        results.extend(self.client.find_many::<JobItem>(JOBS_COLLECTION, filter1, None).await?);
        results.extend(self.client.find_many::<JobItem>(JOBS_COLLECTION, filter2, None).await?);
        results.extend(self.client.find_many::<JobItem>(JOBS_COLLECTION, filter3, None).await?);

        Ok(results)
    }

    async fn get_jobs_without_successor(
        &self,
        job_a_type: JobType,
        job_a_status: JobStatus,
        job_b_type: JobType,
    ) -> Result<Vec<JobItem>, DatabaseError> {
        let pipeline = vec![
            doc! {
                "$match": {
                    "job_type": format!("{:?}", job_a_type),
                    "status": format!("{:?}", job_a_status),
                }
            },
            doc! {
                "$lookup": {
                    "from": JOBS_COLLECTION,
                    "let": { "internal_id": "$internal_id" },
                    "pipeline": [
                        {
                            "$match": {
                                "$expr": {
                                    "$and": [
                                        { "$eq": ["$job_type", format!("{:?}", job_b_type)] },
                                        { "$eq": ["$internal_id", "$$internal_id"] }
                                    ]
                                }
                            }
                        }
                    ],
                    "as": "successor_jobs"
                }
            },
            doc! { "$match": { "successor_jobs": { "$eq": [] } } },
            doc! { "$sort": { "created_at": 1 } },
        ];

        self.client.aggregate::<JobItem, JobItem>(JOBS_COLLECTION, pipeline).await
    }
}
