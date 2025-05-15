use crate::types::batch::Batch;
use crate::types::constant::{
    BLOB_DATA_FILE_NAME, CAIRO_PIE_FILE_NAME, PROGRAM_OUTPUT_FILE_NAME, SNOS_OUTPUT_FILE_NAME,
};
use crate::types::jobs::external_id::ExternalId;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{
    CommonMetadata, DaMetadata, JobMetadata, JobSpecificMetadata, ProvingInputType, ProvingMetadata, SnosMetadata,
    StateUpdateMetadata,
};
use crate::types::jobs::types::{JobStatus, JobType};
use chrono::{SubsecRound, Utc};
use color_eyre::Result;
use std::fs::File;
use std::hash::Hash;
use std::io::Read;
use uuid::Uuid;

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

pub fn build_batch(index: u64, start_block: u64, end_block: u64) -> Batch {
    Batch {
        id: Uuid::new_v4(),
        index,
        size: end_block - start_block + 1,
        start_block,
        end_block,
        is_batch_ready: false,
        squashed_state_updates_path: String::from("path/to/file.json"),
        blob_path: String::from("path/to/file.json"),
        created_at: Utc::now().round_subsecs(0),
        updated_at: Utc::now().round_subsecs(0),
    }
}

pub fn read_blob_from_file(file_path: String) -> Result<String> {
    let mut file = File::open(file_path)?;
    let mut blob = String::new();
    file.read_to_string(&mut blob)?;
    Ok(blob)
}
