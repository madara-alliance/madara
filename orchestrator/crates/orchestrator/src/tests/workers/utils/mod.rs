use std::sync::Arc;

use chrono::{SubsecRound, Utc};
use mockall::predicate::eq;
use uuid::Uuid;

use crate::config::Config;
use crate::constants::{BLOB_DATA_FILE_NAME, CAIRO_PIE_FILE_NAME, PROGRAM_OUTPUT_FILE_NAME, SNOS_OUTPUT_FILE_NAME};
use crate::database::MockDatabase;
use crate::jobs::metadata::{
    CommonMetadata, DaMetadata, JobMetadata, JobSpecificMetadata, ProvingInputTypePath, ProvingMetadata, SnosMetadata,
    StateUpdateMetadata,
};
use crate::jobs::types::{ExternalId, JobItem, JobStatus, JobType};
use crate::jobs::MockJob;

pub fn get_job_item_mock_by_id(id: String, uuid: Uuid) -> JobItem {
    // Parse the ID as a u64 for use in metadata
    let block_number = id.parse::<u64>().unwrap_or(0);

    // Create appropriate metadata for SnosRun job type
    let metadata = JobMetadata {
        common: CommonMetadata::default(),
        specific: JobSpecificMetadata::Snos(SnosMetadata {
            block_number,
            full_output: false,
            cairo_pie_path: Some(format!("{}/{}", block_number, CAIRO_PIE_FILE_NAME)),
            snos_output_path: Some(format!("{}/{}", block_number, SNOS_OUTPUT_FILE_NAME)),
            program_output_path: Some(format!("{}/{}", block_number, PROGRAM_OUTPUT_FILE_NAME)),
            snos_fact: None,
        }),
    };

    JobItem {
        id: uuid,
        internal_id: id.clone(),
        job_type: JobType::SnosRun,
        status: JobStatus::Created,
        external_id: ExternalId::Number(0),
        metadata,
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

        // Create appropriate metadata based on job type
        let metadata = create_metadata_for_job_type(job_type.clone(), i);

        jobs_vec.push(JobItem {
            id: uuid,
            internal_id: i.to_string(),
            job_type: job_type.clone(),
            status: job_status.clone(),
            external_id: ExternalId::Number(0),
            metadata,
            version: 0,
            created_at: Utc::now().round_subsecs(0),
            updated_at: Utc::now().round_subsecs(0),
        })
    }

    jobs_vec
}

/// Helper function to create appropriate metadata based on job type
fn create_metadata_for_job_type(job_type: JobType, block_number: u64) -> JobMetadata {
    match job_type {
        JobType::SnosRun => JobMetadata {
            common: CommonMetadata::default(),
            specific: JobSpecificMetadata::Snos(SnosMetadata {
                block_number,
                full_output: false,
                cairo_pie_path: Some(format!("{}/{}", block_number, CAIRO_PIE_FILE_NAME)),
                snos_output_path: Some(format!("{}/{}", block_number, SNOS_OUTPUT_FILE_NAME)),
                program_output_path: Some(format!("{}/{}", block_number, PROGRAM_OUTPUT_FILE_NAME)),
                snos_fact: Some(String::from("0xdeadbeef")),
            }),
        },
        JobType::DataSubmission => JobMetadata {
            common: CommonMetadata::default(),
            specific: JobSpecificMetadata::Da(DaMetadata {
                block_number,
                blob_data_path: Some(format!("{}/{}", block_number, BLOB_DATA_FILE_NAME)),
                tx_hash: None,
            }),
        },
        JobType::ProofCreation => JobMetadata {
            common: CommonMetadata::default(),
            specific: JobSpecificMetadata::Proving(ProvingMetadata {
                block_number,
                input_path: Some(ProvingInputTypePath::CairoPie(format!("{}/{}", block_number, CAIRO_PIE_FILE_NAME))),
                ensure_on_chain_registration: None,
                download_proof: None,
            }),
        },
        JobType::StateTransition => JobMetadata {
            common: CommonMetadata::default(),
            specific: JobSpecificMetadata::StateUpdate(StateUpdateMetadata {
                blocks_to_settle: vec![block_number],
                snos_output_paths: vec![format!("{}/{}", block_number, SNOS_OUTPUT_FILE_NAME)],
                program_output_paths: vec![format!("{}/{}", block_number, PROGRAM_OUTPUT_FILE_NAME)],
                blob_data_paths: vec![format!("{}/{}", block_number, BLOB_DATA_FILE_NAME)],
                last_failed_block_no: None,
                tx_hashes: Vec::new(),
            }),
        },
        // For any other job types, use a default metadata structure
        _ => JobMetadata {
            common: CommonMetadata::default(),
            specific: JobSpecificMetadata::Snos(SnosMetadata {
                block_number,
                full_output: false,
                cairo_pie_path: None,
                snos_output_path: None,
                program_output_path: None,
                snos_fact: None,
            }),
        },
    }
}

/// Creates and stores both SNOS and DA jobs for a given block number
/// This ensures that the update state worker can find the required jobs
///
/// Arguments:
///
/// `config` - The configuration containing the database client
/// `block_number` - The block number for which to create jobs
/// `job_status` - The status to set for the created jobs
///
/// Returns:
/// A tuple of (SNOS job UUID, DA job UUID)
pub async fn create_and_store_prerequisite_jobs(
    config: Arc<Config>,
    block_number: u64,
    job_status: JobStatus,
) -> color_eyre::Result<(Uuid, Uuid)> {
    // Create SNOS job
    let snos_uuid = Uuid::new_v4();
    let snos_job = JobItem {
        id: snos_uuid,
        internal_id: block_number.to_string(),
        job_type: JobType::SnosRun,
        status: job_status.clone(),
        external_id: ExternalId::Number(0),
        metadata: create_metadata_for_job_type(JobType::SnosRun, block_number),
        version: 0,
        created_at: Utc::now().round_subsecs(0),
        updated_at: Utc::now().round_subsecs(0),
    };

    // Create DA job
    let da_uuid = Uuid::new_v4();
    let da_job = JobItem {
        id: da_uuid,
        internal_id: block_number.to_string(),
        job_type: JobType::DataSubmission,
        status: job_status,
        external_id: ExternalId::Number(0),
        metadata: create_metadata_for_job_type(JobType::DataSubmission, block_number),
        version: 0,
        created_at: Utc::now().round_subsecs(0),
        updated_at: Utc::now().round_subsecs(0),
    };

    // Store jobs in database
    config.database().create_job_item(snos_job).await?;
    config.database().create_job_item(da_job).await?;

    Ok((snos_uuid, da_uuid))
}

pub fn db_checks_proving_worker(id: i32, db: &mut MockDatabase, mock_job: &mut MockJob) {
    // Create a job item with proper metadata for ProofCreation job type
    let uuid = Uuid::new_v4();
    let block_number = id as u64;

    // Create proving metadata with the SNOS fact
    let metadata = JobMetadata {
        common: CommonMetadata::default(),
        specific: JobSpecificMetadata::Proving(ProvingMetadata {
            block_number,
            input_path: Some(ProvingInputTypePath::CairoPie(format!("{}/{}", block_number, CAIRO_PIE_FILE_NAME))),
            ensure_on_chain_registration: Some(format!("0x{:064x}", block_number)), // Add the SNOS fact
            download_proof: None,
        }),
    };

    let job_item = JobItem {
        id: uuid,
        internal_id: id.to_string(),
        job_type: JobType::ProofCreation,
        status: JobStatus::Created,
        external_id: ExternalId::Number(0),
        metadata,
        version: 0,
        created_at: Utc::now().round_subsecs(0),
        updated_at: Utc::now().round_subsecs(0),
    };

    let job_item_cloned = job_item.clone();

    // Check if a proving job already exists for this SNOS job
    db.expect_get_job_by_internal_id_and_type()
        .times(1)
        .with(eq(id.clone().to_string()), eq(JobType::ProofCreation))
        .returning(|_, _| Ok(None));

    // Create the proving job
    mock_job.expect_create_job().times(1).returning(move |_, _, _| Ok(job_item.clone()));

    // Store the job in the database
    db.expect_create_job_item()
        .times(1)
        .withf(move |item| item.internal_id == id.clone().to_string())
        .returning(move |_| Ok(job_item_cloned.clone()));
}
