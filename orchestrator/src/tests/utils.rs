use crate::types::batch::{AggregatorBatch, AggregatorBatchStatus};
use chrono::{SubsecRound, Utc};
use rstest::fixture;
use uuid::Uuid;

use crate::tests::config::{ConfigType, MockType, TestConfigBuilder, TestConfigBuilderReturns};
use crate::tests::jobs::snos_job::SNOS_PATHFINDER_RPC_URL_ENV;
use crate::types::constant::{
    BLOB_DATA_FILE_NAME, CAIRO_PIE_FILE_NAME, PROGRAM_OUTPUT_FILE_NAME, SNOS_OUTPUT_FILE_NAME,
};
use crate::types::jobs::external_id::ExternalId;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{
    CommonMetadata, DaMetadata, JobMetadata, JobSpecificMetadata, ProvingInputType, ProvingMetadata, SettlementContext,
    SettlementContextData, SnosMetadata, StateUpdateMetadata,
};
use crate::types::jobs::types::{JobStatus, JobType};
use color_eyre::Result;
use starknet_core::types::{Felt, StateUpdate};
use std::fs::File;
use std::io::Read;
use url::Url;

pub fn build_job_item(job_type: JobType, job_status: JobStatus, internal_id: u64) -> JobItem {
    let metadata = match job_type {
        JobType::StateTransition => JobMetadata {
            common: CommonMetadata::default(),
            specific: JobSpecificMetadata::StateUpdate(StateUpdateMetadata {
                snos_output_paths: vec![format!("{}/{}", internal_id, SNOS_OUTPUT_FILE_NAME)],
                program_output_paths: vec![format!("{}/{}", internal_id, PROGRAM_OUTPUT_FILE_NAME)],
                blob_data_paths: vec![format!("{}/{}", internal_id, BLOB_DATA_FILE_NAME)],
                tx_hashes: Vec::new(),
                context: SettlementContext::Block(SettlementContextData {
                    to_settle: vec![internal_id],
                    last_failed: None,
                }),
            }),
        },
        JobType::SnosRun => JobMetadata {
            common: CommonMetadata::default(),
            specific: JobSpecificMetadata::Snos(SnosMetadata {
                snos_batch_index: 1,
                start_block: internal_id,
                end_block: internal_id,
                num_blocks: 1,
                full_output: true,
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

#[fixture]
pub fn build_batch(
    #[default(1)] index: u64,
    #[default(100)] start_block: u64,
    #[default(200)] end_block: u64,
) -> AggregatorBatch {
    AggregatorBatch {
        id: Uuid::new_v4(),
        index,
        num_snos_batches: 5,
        start_snos_batch: 10,
        end_snos_batch: 15,
        num_blocks: end_block - start_block + 1,
        start_block,
        end_block,
        is_batch_ready: false,
        squashed_state_updates_path: String::from("path/to/file.json"),
        blob_path: String::from("path/to/file.json"),
        created_at: Utc::now().round_subsecs(0),
        updated_at: Utc::now().round_subsecs(0),
        bucket_id: String::from("ABCD1234"),
        status: AggregatorBatchStatus::Open,
        starknet_version: String::from("0.13.2"),
    }
}

pub async fn build_test_config_with_real_provider() -> Result<TestConfigBuilderReturns> {
    let pathfinder_url: Url = match std::env::var(SNOS_PATHFINDER_RPC_URL_ENV) {
        Ok(url) => url.parse()?,
        Err(_) => {
            return Err(color_eyre::eyre::eyre!("{} environment variable is not set", SNOS_PATHFINDER_RPC_URL_ENV));
        }
    };

    Ok(TestConfigBuilder::new().configure_rpc_url(ConfigType::Mock(MockType::RpcUrl(pathfinder_url))).build().await)
}

pub fn read_felt_vec_from_file(file_path: &str) -> Result<Vec<Felt>> {
    serde_json::from_str(&read_file_to_string(file_path)?)
        .map_err(|e| color_eyre::eyre::eyre!("Failed to parse felt vector file: {}", e))
}

pub fn read_state_updates_vec_from_file(file_path: &str) -> Result<Vec<StateUpdate>> {
    serde_json::from_str(&read_file_to_string(file_path)?)
        .map_err(|e| color_eyre::eyre::eyre!("Failed to parse state update vector file: {}", e))
}

pub fn read_state_update_from_file(file_path: &str) -> Result<StateUpdate> {
    serde_json::from_str(&read_file_to_string(file_path)?)
        .map_err(|e| color_eyre::eyre::eyre!("Failed to parse state update file: {}", e))
}

pub fn read_file_to_string(file_path: &str) -> Result<String> {
    let mut string = String::new();
    let mut file = File::open(file_path)?;
    file.read_to_string(&mut string)?;
    Ok(string)
}
