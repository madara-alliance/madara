use async_trait::async_trait;
use chrono::{SubsecRound, Utc};
use mongodb::bson::{self, doc, Bson};
use mongodb::options::{FindOneAndUpdateOptions, FindOptions, ReturnDocument};
use std::sync::Arc;
use tracing::{debug, trace, warn};
use uuid::Uuid;

use super::r#trait::WorkerRepository;
use crate::core::client::database::constant::JOBS_COLLECTION;
use crate::core::client::database::error::DatabaseError;
use crate::core::client::database::mongo_client::MongoClient;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::types::{JobStatus, JobType};

pub struct MongoWorkerRepository {
    client: Arc<MongoClient>,
}

impl MongoWorkerRepository {
    pub fn new(client: Arc<MongoClient>) -> Self {
        Self { client }
    }

    /// Build common filter for available jobs (not claimed, available_at passed)
    fn available_job_filter() -> mongodb::bson::Document {
        doc! {
            "$and": [
                {
                    "$or": [
                        { "available_at": { "$exists": false } },
                        { "available_at": null },
                        { "available_at": { "$lte": Utc::now().trunc_subsecs(3) } }
                    ]
                },
                {
                    "$or": [
                        { "claimed_by": { "$exists": false } },
                        { "claimed_by": null }
                    ]
                }
            ]
        }
    }
}

#[async_trait]
impl WorkerRepository for MongoWorkerRepository {
    // =========================================================================
    // Job Discovery
    // =========================================================================

    async fn get_processable_job(&self, job_type: &JobType) -> Result<Option<JobItem>, DatabaseError> {
        let mut filter = doc! {
            "job_type": bson::to_bson(job_type)?,
            "status": {
                "$in": [
                    bson::to_bson(&JobStatus::Created)?,
                    bson::to_bson(&JobStatus::PendingRetryProcessing)?,
                ]
            },
        };
        filter.extend(Self::available_job_filter());

        let options = FindOptions::builder()
            .sort(doc! { "status": -1, "created_at": 1 }) // PendingRetry first, then FIFO
            .limit(1)
            .build();

        let mut results: Vec<JobItem> = self.client.find_many(JOBS_COLLECTION, filter, Some(options)).await?;

        Ok(results.pop())
    }

    async fn get_verifiable_job(&self, job_type: &JobType) -> Result<Option<JobItem>, DatabaseError> {
        let mut filter = doc! {
            "job_type": bson::to_bson(job_type)?,
            "status": {
                "$in": [
                    bson::to_bson(&JobStatus::Processed)?,
                    bson::to_bson(&JobStatus::PendingRetryVerification)?,
                ]
            },
        };
        filter.extend(Self::available_job_filter());

        let options = FindOptions::builder().sort(doc! { "status": -1, "created_at": 1 }).limit(1).build();

        let mut results: Vec<JobItem> = self.client.find_many(JOBS_COLLECTION, filter, Some(options)).await?;

        Ok(results.pop())
    }

    // =========================================================================
    // Atomic Claiming
    // =========================================================================

    async fn claim_for_processing(
        &self,
        job_type: &JobType,
        orchestrator_id: &str,
    ) -> Result<Option<JobItem>, DatabaseError> {
        let now = Utc::now().trunc_subsecs(3);

        let mut filter = doc! {
            "job_type": bson::to_bson(job_type)?,
            "status": {
                "$in": [
                    bson::to_bson(&JobStatus::PendingRetryProcessing)?,
                    bson::to_bson(&JobStatus::Created)?,
                ]
            },
        };
        filter.extend(Self::available_job_filter());

        let update = doc! {
            "$set": {
                "status": bson::to_bson(&JobStatus::LockedForProcessing)?,
                "claimed_by": orchestrator_id,
                "updated_at": now,
            },
            "$inc": { "version": 1 }
        };

        // PendingRetryProcessing > Created alphabetically, so -1 gives retry priority
        let options = FindOneAndUpdateOptions::builder()
            .return_document(ReturnDocument::After)
            .sort(doc! { "status": -1, "created_at": 1 })
            .build();

        let result = self.client.find_one_and_update::<JobItem>(JOBS_COLLECTION, filter, update, options).await?;

        if let Some(ref job) = result {
            debug!(
                job_id = %job.id,
                job_type = ?job_type,
                orchestrator = orchestrator_id,
                "Claimed job for processing"
            );
        }

        Ok(result)
    }

    async fn claim_for_verification(
        &self,
        job_type: &JobType,
        orchestrator_id: &str,
    ) -> Result<Option<JobItem>, DatabaseError> {
        let now = Utc::now().trunc_subsecs(3);

        let mut filter = doc! {
            "job_type": bson::to_bson(job_type)?,
            "status": bson::to_bson(&JobStatus::Processed)?,
        };
        filter.extend(Self::available_job_filter());

        let update = doc! {
            "$set": {
                "claimed_by": orchestrator_id,
                "updated_at": now,
            },
            "$inc": { "version": 1 }
        };

        let options = FindOneAndUpdateOptions::builder()
            .return_document(ReturnDocument::After)
            .sort(doc! { "created_at": 1 }) // FIFO
            .build();

        let result = self.client.find_one_and_update::<JobItem>(JOBS_COLLECTION, filter, update, options).await?;

        if let Some(ref job) = result {
            debug!(
                job_id = %job.id,
                job_type = ?job_type,
                orchestrator = orchestrator_id,
                "Claimed job for verification"
            );
        }

        Ok(result)
    }

    async fn release_claim(&self, job_id: Uuid, delay_seconds: Option<u64>) -> Result<JobItem, DatabaseError> {
        let now = Utc::now().trunc_subsecs(3);
        let filter = doc! { "id": job_id };

        let mut set_doc = doc! { "updated_at": now };
        let mut unset_doc = doc! { "claimed_by": "" };

        if let Some(delay) = delay_seconds {
            let available_at = now + chrono::Duration::seconds(delay as i64);
            set_doc.insert("available_at", Bson::DateTime(available_at.into()));
        } else {
            unset_doc.insert("available_at", "");
        }

        let update = doc! {
            "$set": set_doc,
            "$unset": unset_doc,
            "$inc": { "version": 1 }
        };

        let options = FindOneAndUpdateOptions::builder().return_document(ReturnDocument::After).build();

        self.client
            .find_one_and_update::<JobItem>(JOBS_COLLECTION, filter, update, options)
            .await?
            .ok_or_else(|| DatabaseError::NoUpdateFound(format!("Job not found: {}", job_id)))
    }

    // =========================================================================
    // Orphan Detection
    // =========================================================================

    async fn get_orphaned_jobs(&self, job_type: &JobType, timeout_seconds: u64) -> Result<Vec<JobItem>, DatabaseError> {
        let cutoff = Utc::now().trunc_subsecs(3) - chrono::Duration::seconds(timeout_seconds as i64);

        let filter = doc! {
            "job_type": bson::to_bson(job_type)?,
            "status": {
                "$in": [
                    bson::to_bson(&JobStatus::LockedForProcessing)?,
                    bson::to_bson(&JobStatus::LockedForVerification)?,
                ]
            },
            "updated_at": { "$lt": cutoff }
        };

        let jobs: Vec<JobItem> = self.client.find_many(JOBS_COLLECTION, filter, None).await?;

        if !jobs.is_empty() {
            warn!(
                job_type = ?job_type,
                count = jobs.len(),
                timeout_seconds,
                "Found orphaned jobs"
            );
        }

        Ok(jobs)
    }

    async fn get_orphaned_verification_jobs(
        &self,
        job_type: &JobType,
        timeout_seconds: u64,
    ) -> Result<Vec<JobItem>, DatabaseError> {
        let cutoff = Utc::now().trunc_subsecs(3) - chrono::Duration::seconds(timeout_seconds as i64);

        let filter = doc! {
            "job_type": bson::to_bson(job_type)?,
            "status": bson::to_bson(&JobStatus::Processed)?,
            "claimed_by": { "$ne": null },
            "updated_at": { "$lt": cutoff }
        };

        let jobs: Vec<JobItem> = self.client.find_many(JOBS_COLLECTION, filter, None).await?;

        if !jobs.is_empty() {
            warn!(
                job_type = ?job_type,
                count = jobs.len(),
                timeout_seconds,
                "Found orphaned verification jobs"
            );
        }

        Ok(jobs)
    }

    // =========================================================================
    // Counting
    // =========================================================================

    async fn count_by_type_and_status(&self, job_type: &JobType, status: JobStatus) -> Result<u64, DatabaseError> {
        let filter = doc! {
            "job_type": bson::to_bson(job_type)?,
            "status": bson::to_bson(&status)?
        };
        self.client.count::<JobItem>(JOBS_COLLECTION, filter).await
    }

    async fn count_by_type_and_statuses(
        &self,
        job_type: &JobType,
        statuses: &[JobStatus],
    ) -> Result<u64, DatabaseError> {
        let statuses_bson: Vec<Bson> = statuses.iter().map(|s| bson::to_bson(s).unwrap()).collect();

        let filter = doc! {
            "job_type": bson::to_bson(job_type)?,
            "status": { "$in": statuses_bson }
        };
        self.client.count::<JobItem>(JOBS_COLLECTION, filter).await
    }

    async fn count_claimed(&self, orchestrator_id: &str) -> Result<u64, DatabaseError> {
        let filter = doc! { "claimed_by": orchestrator_id };
        let count = self.client.count::<JobItem>(JOBS_COLLECTION, filter).await?;
        debug!(orchestrator = orchestrator_id, count, "Counted claimed jobs");
        Ok(count)
    }

    async fn count_claimed_by_type(&self, orchestrator_id: &str, job_type: &JobType) -> Result<u64, DatabaseError> {
        let filter = doc! {
            "claimed_by": orchestrator_id,
            "job_type": bson::to_bson(job_type)?
        };
        let count = self.client.count::<JobItem>(JOBS_COLLECTION, filter).await?;
        trace!(
            orchestrator = orchestrator_id,
            job_type = ?job_type,
            count,
            "Counted claimed jobs by type"
        );
        Ok(count)
    }
}
