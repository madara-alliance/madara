use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use cairo_vm::types::layout_name::LayoutName;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use cairo_vm::Felt252;
use chrono::{SubsecRound, Utc};
use color_eyre::Result;
use prove_block::prove_block;
use starknet_os::io::output::StarknetOsOutput;
use tempfile::NamedTempFile;
use thiserror::Error;
use uuid::Uuid;

use super::constants::{JOB_METADATA_SNOS_BLOCK, JOB_METADATA_SNOS_FACT};
use super::{JobError, OtherError};
use crate::config::Config;
use crate::constants::{CAIRO_PIE_FILE_NAME, PROGRAM_OUTPUT_FILE_NAME, SNOS_OUTPUT_FILE_NAME};
use crate::data_storage::DataStorage;
use crate::jobs::snos_job::error::FactError;
use crate::jobs::snos_job::fact_info::get_fact_info;
use crate::jobs::types::{JobItem, JobStatus, JobType, JobVerificationStatus};
use crate::jobs::Job;

pub mod error;
pub mod fact_info;
pub mod fact_node;
pub mod fact_topology;

#[derive(Error, Debug, PartialEq)]
pub enum SnosError {
    #[error("Block numbers to run must be specified (snos job #{internal_id:?})")]
    UnspecifiedBlockNumber { internal_id: String },
    #[error("No block numbers found (snos job #{internal_id:?})")]
    BlockNumberNotFound { internal_id: String },
    #[error("Invalid specified block number \"{block_number:?}\" (snos job #{internal_id:?})")]
    InvalidBlockNumber { internal_id: String, block_number: String },

    #[error("Could not serialize the Cairo Pie (snos job #{internal_id:?}): {message}")]
    CairoPieUnserializable { internal_id: String, message: String },
    #[error("Could not store the Cairo Pie (snos job #{internal_id:?}): {message}")]
    CairoPieUnstorable { internal_id: String, message: String },

    #[error("Could not serialize the Snos Output (snos job #{internal_id:?}): {message}")]
    SnosOutputUnserializable { internal_id: String, message: String },
    #[error("Could not serialize the Program Output (snos job #{internal_id:?}): {message}")]
    ProgramOutputUnserializable { internal_id: String, message: String },
    #[error("Could not store the Snos output (snos job #{internal_id:?}): {message}")]
    SnosOutputUnstorable { internal_id: String, message: String },
    #[error("Could not store the Program output (snos job #{internal_id:?}): {message}")]
    ProgramOutputUnstorable { internal_id: String, message: String },

    // ProveBlockError from Snos is not usable with #[from] since it does not implement PartialEq.
    // Instead, we convert it to string & pass it into the [SnosExecutionError] error.
    #[error("Error while running SNOS (snos job #{internal_id:?}): {message}")]
    SnosExecutionError { internal_id: String, message: String },

    #[error("Error when calculating fact info: {0}")]
    FactCalculationError(#[from] FactError),

    #[error("Other error: {0}")]
    Other(#[from] OtherError),
}

pub struct SnosJob;

#[async_trait]
impl Job for SnosJob {
    #[tracing::instrument(fields(category = "snos"), skip(self, _config, metadata))]
    async fn create_job(
        &self,
        _config: Arc<Config>,
        internal_id: String,
        metadata: HashMap<String, String>,
    ) -> Result<JobItem, JobError> {
        let mut metadata = metadata;
        metadata.insert(JOB_METADATA_SNOS_BLOCK.to_string(), internal_id.clone());
        Ok(JobItem {
            id: Uuid::new_v4(),
            internal_id: internal_id.clone(),
            job_type: JobType::SnosRun,
            status: JobStatus::Created,
            external_id: String::new().into(),
            metadata,
            version: 0,
            created_at: Utc::now().round_subsecs(0),
            updated_at: Utc::now().round_subsecs(0),
        })
    }

    #[tracing::instrument(fields(category = "snos"), skip(self, config))]
    async fn process_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<String, JobError> {
        let block_number = self.get_block_number_from_metadata(job)?;

        let snos_url = config.snos_url().to_string();
        let snos_url = snos_url.trim_end_matches('/');
        let (cairo_pie, snos_output) =
            prove_block(block_number, snos_url, LayoutName::all_cairo, false).await.map_err(|e| {
                SnosError::SnosExecutionError { internal_id: job.internal_id.clone(), message: e.to_string() }
            })?;

        let fact_info = get_fact_info(&cairo_pie, None)?;
        let program_output = fact_info.program_output;

        // snos output = output returned by SNOS
        // program output = output written on the output segment
        self.store(config.storage(), &job.internal_id, block_number, cairo_pie, snos_output, program_output).await?;

        // store the fact
        job.metadata.insert(JOB_METADATA_SNOS_FACT.into(), fact_info.fact.to_string());

        Ok(block_number.to_string())
    }

    #[tracing::instrument(fields(category = "snos"), skip(self, _config))]
    async fn verify_job(&self, _config: Arc<Config>, _job: &mut JobItem) -> Result<JobVerificationStatus, JobError> {
        // No need for verification as of now. If we later on decide to outsource SNOS run
        // to another service, verify_job can be used to poll on the status of the job
        Ok(JobVerificationStatus::Verified)
    }

    fn max_process_attempts(&self) -> u64 {
        1
    }

    fn max_verification_attempts(&self) -> u64 {
        1
    }

    fn verification_polling_delay_seconds(&self) -> u64 {
        1
    }
}

impl SnosJob {
    /// Get the block number that needs to be run with SNOS for the current
    /// job.
    fn get_block_number_from_metadata(&self, job: &JobItem) -> Result<u64, SnosError> {
        let block_number: u64 = job
            .metadata
            .get(JOB_METADATA_SNOS_BLOCK)
            .ok_or(SnosError::UnspecifiedBlockNumber { internal_id: job.internal_id.clone() })?
            .parse()
            .map_err(|_| SnosError::InvalidBlockNumber {
                internal_id: job.internal_id.clone(),
                block_number: job.metadata[JOB_METADATA_SNOS_BLOCK].clone(),
            })?;

        Ok(block_number)
    }

    /// Stores the [CairoPie] and the [StarknetOsOutput] in the Data Storage.
    /// The paths will be:
    ///     - [block_number]/cairo_pie.zip
    ///     - [block_number]/snos_output.json
    async fn store(
        &self,
        data_storage: &dyn DataStorage,
        internal_id: &str,
        block_number: u64,
        cairo_pie: CairoPie,
        snos_output: StarknetOsOutput,
        program_output: Vec<Felt252>,
    ) -> Result<(), SnosError> {
        let cairo_pie_key = format!("{block_number}/{CAIRO_PIE_FILE_NAME}");
        let cairo_pie_zip_bytes = self.cairo_pie_to_zip_bytes(cairo_pie).await.map_err(|e| {
            SnosError::CairoPieUnserializable { internal_id: internal_id.to_string(), message: e.to_string() }
        })?;
        data_storage.put_data(cairo_pie_zip_bytes, &cairo_pie_key).await.map_err(|e| {
            SnosError::CairoPieUnstorable { internal_id: internal_id.to_string(), message: e.to_string() }
        })?;

        let snos_output_key = format!("{block_number}/{SNOS_OUTPUT_FILE_NAME}");
        let snos_output_json = serde_json::to_vec(&snos_output).map_err(|e| SnosError::SnosOutputUnserializable {
            internal_id: internal_id.to_string(),
            message: e.to_string(),
        })?;
        data_storage.put_data(snos_output_json.into(), &snos_output_key).await.map_err(|e| {
            SnosError::SnosOutputUnstorable { internal_id: internal_id.to_string(), message: e.to_string() }
        })?;

        let program_output: Vec<[u8; 32]> = program_output.iter().map(|f| f.to_bytes_be()).collect();
        let encoded_data = bincode::serialize(&program_output).map_err(|e| SnosError::ProgramOutputUnserializable {
            internal_id: internal_id.to_string(),
            message: e.to_string(),
        })?;
        let program_output_key = format!("{block_number}/{PROGRAM_OUTPUT_FILE_NAME}");
        data_storage.put_data(encoded_data.into(), &program_output_key).await.map_err(|e| {
            SnosError::ProgramOutputUnstorable { internal_id: internal_id.to_string(), message: e.to_string() }
        })?;

        Ok(())
    }

    /// Converts the [CairoPie] input as a zip file and returns it as [Bytes].
    async fn cairo_pie_to_zip_bytes(&self, cairo_pie: CairoPie) -> Result<Bytes> {
        let mut cairo_pie_zipfile = NamedTempFile::new()?;
        cairo_pie.write_zip_file(cairo_pie_zipfile.path())?;
        let cairo_pie_zip_bytes = self.tempfile_to_bytes(&mut cairo_pie_zipfile)?;
        cairo_pie_zipfile.close()?;
        Ok(cairo_pie_zip_bytes)
    }

    /// Converts a [NamedTempFile] to [Bytes].
    fn tempfile_to_bytes(&self, tmp_file: &mut NamedTempFile) -> Result<Bytes> {
        let mut buffer = Vec::new();
        tmp_file.as_file_mut().read_to_end(&mut buffer)?;
        Ok(Bytes::from(buffer))
    }
}
