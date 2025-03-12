use chrono::{SubsecRound, Utc};
use uuid::Uuid;

use crate::constants::{BLOB_DATA_FILE_NAME, CAIRO_PIE_FILE_NAME, PROGRAM_OUTPUT_FILE_NAME, SNOS_OUTPUT_FILE_NAME};
use crate::jobs::metadata::{
    CommonMetadata, DaMetadata, JobMetadata, JobSpecificMetadata, ProvingInputType, ProvingMetadata, SnosMetadata,
    StateUpdateMetadata,
};
use crate::jobs::types::{ExternalId, JobItem, JobStatus, JobType};

// Test Util Functions
// ==========================================

pub fn build_job_item(job_type: JobType, job_status: JobStatus, internal_id: u64) -> JobItem {
    let metadata = match job_type {
        JobType::StateTransition => JobMetadata {
            common: CommonMetadata::default(),
            specific: JobSpecificMetadata::StateUpdate(StateUpdateMetadata {
                blocks_to_settle: vec![internal_id],
                snos_output_paths: vec![format!("{}/{}", internal_id, SNOS_OUTPUT_FILE_NAME)],
                program_output_paths: vec![format!("{}/{}", internal_id, PROGRAM_OUTPUT_FILE_NAME)],
                blob_data_paths: vec![format!("{}/{}", internal_id, BLOB_DATA_FILE_NAME)],
                last_failed_block_no: None,
                tx_hashes: Vec::new(),
            }),
        },
        JobType::SnosRun => JobMetadata {
            common: CommonMetadata::default(),
            specific: JobSpecificMetadata::Snos(SnosMetadata {
                block_number: internal_id,
                cairo_pie_path: Some(format!("{}/{}", internal_id, CAIRO_PIE_FILE_NAME)),
                snos_output_path: Some(format!("{}/{}", internal_id, SNOS_OUTPUT_FILE_NAME)),
                program_output_path: Some(format!("{}/{}", internal_id, PROGRAM_OUTPUT_FILE_NAME)),
                ..Default::default()
            }),
        },
        JobType::ProofCreation => JobMetadata {
            common: CommonMetadata::default(),
            specific: JobSpecificMetadata::Proving(ProvingMetadata {
                block_number: internal_id,
                input_path: Some(ProvingInputType::CairoPie(format!("{}/{}", internal_id, CAIRO_PIE_FILE_NAME))),
                ..Default::default()
            }),
        },
        JobType::DataSubmission => JobMetadata {
            common: CommonMetadata::default(),
            specific: JobSpecificMetadata::Da(DaMetadata {
                block_number: internal_id,
                blob_data_path: Some(format!("{}/{}", internal_id, BLOB_DATA_FILE_NAME)),
                tx_hash: None,
            }),
        },
        _ => panic!("Invalid job type"),
    };

    JobItem {
        id: Uuid::new_v4(),
        internal_id: internal_id.to_string(),
        job_type,
        status: job_status,
        external_id: ExternalId::Number(0),
        metadata,
        version: 0,
        created_at: Utc::now().round_subsecs(0),
        updated_at: Utc::now().round_subsecs(0),
    }
}
