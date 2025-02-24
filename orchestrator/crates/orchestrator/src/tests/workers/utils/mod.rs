use std::collections::HashMap;

use chrono::{SubsecRound, Utc};
use mockall::predicate::eq;
use uuid::Uuid;

use crate::database::MockDatabase;
use crate::jobs::types::{ExternalId, JobItem, JobStatus, JobType};
use crate::jobs::MockJob;

pub fn get_job_item_mock_by_id(id: String, uuid: Uuid) -> JobItem {
    JobItem {
        id: uuid,
        internal_id: id.clone(),
        job_type: JobType::SnosRun,
        status: JobStatus::Created,
        external_id: ExternalId::Number(0),
        metadata: HashMap::new(),
        version: 0,
        created_at: Utc::now().round_subsecs(0),
        updated_at: Utc::now().round_subsecs(0),
    }
}

/// Function to get the vector of JobItems with mock IDs
///
/// Arguments :
///
/// `job_type` : Type of job you want to create the vector for.
///
/// `job_status` : State of the job you want to create the vector for.
///
/// `number_of_jobs` : Number of jobs (length of the vector you need).
///
/// `start_index` : Start index of the `internal_id` for the JobItem in the vector.
pub fn get_job_by_mock_id_vector(
    job_type: JobType,
    job_status: JobStatus,
    number_of_jobs: u64,
    start_index: u64,
) -> Vec<JobItem> {
    let mut jobs_vec: Vec<JobItem> = Vec::new();

    for i in start_index..number_of_jobs + start_index {
        let uuid = Uuid::new_v4();
        jobs_vec.push(JobItem {
            id: uuid,
            internal_id: i.to_string(),
            job_type: job_type.clone(),
            status: job_status.clone(),
            external_id: ExternalId::Number(0),
            metadata: HashMap::new(),
            version: 0,
            created_at: Utc::now().round_subsecs(0),
            updated_at: Utc::now().round_subsecs(0),
        })
    }

    jobs_vec
}

pub fn db_checks_proving_worker(id: i32, db: &mut MockDatabase, mock_job: &mut MockJob) {
    fn get_job_item_mock_by_id(id: i32) -> JobItem {
        let uuid = Uuid::new_v4();
        JobItem {
            id: uuid,
            internal_id: id.to_string(),
            job_type: JobType::ProofCreation,
            status: JobStatus::Created,
            external_id: ExternalId::Number(0),
            metadata: HashMap::new(),
            version: 0,
            created_at: Utc::now().round_subsecs(0),
            updated_at: Utc::now().round_subsecs(0),
        }
    }

    db.expect_get_job_by_internal_id_and_type()
        .times(1)
        .with(eq(id.clone().to_string()), eq(JobType::ProofCreation))
        .returning(|_, _| Ok(None));

    let job_item = get_job_item_mock_by_id(id);
    let job_item_cloned = job_item.clone();

    mock_job.expect_create_job().times(1).returning(move |_, _, _| Ok(job_item.clone()));

    db.expect_create_job()
        .times(1)
        .withf(move |item| item.internal_id == id.clone().to_string())
        .returning(move |_| Ok(job_item_cloned.clone()));
}
