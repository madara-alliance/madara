use std::io::Read;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use cairo_vm::types::layout_name::LayoutName;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use cairo_vm::Felt252;
use chrono::{SubsecRound, Utc};
use color_eyre::eyre::eyre;
use color_eyre::Result;
use prove_block::prove_block;
use starknet_os::io::output::StarknetOsOutput;
use tempfile::NamedTempFile;
use thiserror::Error;
use uuid::Uuid;

use super::{JobError, OtherError};
use crate::config::Config;
use crate::data_storage::DataStorage;
use crate::helpers;
use crate::jobs::metadata::{JobMetadata, JobSpecificMetadata, SnosMetadata};
use crate::jobs::snos_job::error::FactError;
use crate::jobs::snos_job::fact_info::get_fact_info;
use crate::jobs::types::{JobItem, JobStatus, JobType, JobVerificationStatus};
use crate::jobs::Job;

pub mod error;
pub mod fact_info;
pub mod fact_node;
pub mod fact_topology;

pub const COMPILED_OS: &[u8] = include_bytes!("../../../../../build/os_latest.json");

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
    #[tracing::instrument(fields(category = "snos"), skip(self, _config, metadata), ret, err)]
    async fn create_job(
        &self,
        _config: Arc<Config>,
        internal_id: String,
        metadata: JobMetadata,
    ) -> Result<JobItem, JobError> {
        tracing::info!(
            log_type = "starting",
            category = "snos",
            function_type = "create_job",
            block_no = %internal_id,
            "SNOS job creation started."
        );

        let job_item = JobItem {
            id: Uuid::new_v4(),
            internal_id: internal_id.clone(),
            job_type: JobType::SnosRun,
            status: JobStatus::Created,
            external_id: String::new().into(),
            metadata,
            version: 0,
            created_at: Utc::now().round_subsecs(0),
            updated_at: Utc::now().round_subsecs(0),
        };

        tracing::info!(
            log_type = "completed",
            category = "snos",
            function_type = "create_job",
            block_no = %internal_id,
            "SNOS job creation completed."
        );
        Ok(job_item)
    }

    #[tracing::instrument(fields(category = "snos"), skip(self, config), ret, err)]
    async fn process_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<String, JobError> {
        let internal_id = job.internal_id.clone();
        tracing::info!(
            log_type = "starting",
            category = "snos",
            function_type = "process_job",
            job_id = ?job.id,
            block_no = %internal_id,
            "SNOS job processing started."
        );

        // Get SNOS metadata
        let snos_metadata: SnosMetadata = job.metadata.specific.clone().try_into().map_err(|e| {
            tracing::error!(job_id = %job.internal_id, error = %e, "Invalid metadata type for SNOS job");
            JobError::Other(OtherError(e))
        })?;

        // Get block number from metadata
        let block_number = snos_metadata.block_number;
        tracing::debug!(job_id = %job.internal_id, block_number = %block_number, "Retrieved block number from metadata");

        let snos_url = config.snos_config().rpc_for_snos.to_string();
        let snos_url = snos_url.trim_end_matches('/');
        tracing::debug!(job_id = %job.internal_id, "Calling prove_block function");

        let (cairo_pie, snos_output) =
            prove_block(COMPILED_OS, block_number, snos_url, LayoutName::all_cairo, snos_metadata.full_output)
                .await
                .map_err(|e| {
                    tracing::error!(job_id = %job.internal_id, error = %e, "SNOS execution failed");
                    SnosError::SnosExecutionError { internal_id: job.internal_id.clone(), message: e.to_string() }
                })?;
        tracing::debug!(job_id = %job.internal_id, "prove_block function completed successfully");

        let fact_info = get_fact_info(&cairo_pie, None)?;
        let program_output = fact_info.program_output;
        tracing::debug!(job_id = %job.internal_id, "Fact info calculated successfully");

        tracing::debug!(job_id = %job.internal_id, "Storing SNOS outputs");
        self.store(internal_id.clone(), config.storage(), &snos_metadata, cairo_pie, snos_output, program_output)
            .await?;

        // Update the metadata with new paths and fact info
        if let JobSpecificMetadata::Snos(metadata) = &mut job.metadata.specific {
            metadata.snos_fact = Some(fact_info.fact.to_string());
        }

        tracing::info!(
            log_type = "completed",
            category = "snos",
            function_type = "process_job",
            job_id = ?job.id,
            block_no = %block_number,
            "SNOS job processed successfully."
        );

        Ok(block_number.to_string())
    }

    #[tracing::instrument(fields(category = "snos"), skip(self, _config), ret, err)]
    async fn verify_job(&self, _config: Arc<Config>, job: &mut JobItem) -> Result<JobVerificationStatus, JobError> {
        let internal_id = job.internal_id.clone();
        tracing::info!(log_type = "starting", category = "snos", function_type = "verify_job", job_id = %job.id,  block_no = %internal_id, "SNOS job verification started.");
        // No need for verification as of now. If we later on decide to outsource SNOS run
        // to another service, verify_job can be used to poll on the status of the job
        tracing::info!(log_type = "completed", category = "snos", function_type = "verify_job", job_id = %job.id,  block_no = %internal_id, "SNOS job verification completed.");
        Ok(JobVerificationStatus::Verified)
    }

    fn max_process_attempts(&self) -> u16 {
        1
    }

    fn max_verification_attempts(&self) -> u16 {
        1
    }

    fn verification_polling_delay_seconds(&self) -> u64 {
        1
    }

    fn job_processing_lock(&self, config: Arc<Config>) -> Option<Arc<helpers::JobProcessingState>> {
        config.processing_locks().snos_job_processing_lock.clone()
    }
}

impl SnosJob {
    /// Stores the [CairoPie] and the [StarknetOsOutput] in the Data Storage.
    /// The paths will be:
    ///     - [block_number]/cairo_pie.zip
    ///     - [block_number]/snos_output.json
    async fn store(
        &self,
        internal_id: String,
        data_storage: &dyn DataStorage,
        snos_metadata: &SnosMetadata,
        cairo_pie: CairoPie,
        snos_output: StarknetOsOutput,
        program_output: Vec<Felt252>,
    ) -> Result<(), SnosError> {
        // Get storage paths from metadata
        let cairo_pie_key = snos_metadata
            .cairo_pie_path
            .as_ref()
            .ok_or_else(|| SnosError::Other(OtherError(eyre!("Cairo Pie path not found in metadata"))))?;

        let snos_output_key = snos_metadata
            .snos_output_path
            .as_ref()
            .ok_or_else(|| SnosError::Other(OtherError(eyre!("SNOS output path not found in metadata"))))?;

        let program_output_key = snos_metadata
            .program_output_path
            .as_ref()
            .ok_or_else(|| SnosError::Other(OtherError(eyre!("Program output path not found in metadata"))))?;

        // Store Cairo Pie
        {
            let cairo_pie_zip_bytes = self.cairo_pie_to_zip_bytes(cairo_pie).await.map_err(|e| {
                SnosError::CairoPieUnserializable { internal_id: internal_id.clone(), message: e.to_string() }
            })?;
            data_storage.put_data(cairo_pie_zip_bytes, cairo_pie_key).await.map_err(|e| {
                SnosError::CairoPieUnstorable { internal_id: internal_id.clone(), message: e.to_string() }
            })?;
        }

        // Store SNOS Output
        {
            let snos_output_json = serde_json::to_vec(&snos_output).map_err(|e| {
                SnosError::SnosOutputUnserializable { internal_id: internal_id.clone(), message: e.to_string() }
            })?;
            data_storage.put_data(snos_output_json.into(), snos_output_key).await.map_err(|e| {
                SnosError::SnosOutputUnstorable { internal_id: internal_id.clone(), message: e.to_string() }
            })?;
        }

        // Store Program Output
        {
            let program_output: Vec<[u8; 32]> = program_output.iter().map(|f| f.to_bytes_be()).collect();
            let encoded_data = bincode::serialize(&program_output).map_err(|e| {
                SnosError::ProgramOutputUnserializable { internal_id: internal_id.clone(), message: e.to_string() }
            })?;
            data_storage.put_data(encoded_data.into(), program_output_key).await.map_err(|e| {
                SnosError::ProgramOutputUnstorable { internal_id: internal_id.clone(), message: e.to_string() }
            })?;
        }

        Ok(())
    }

    /// Converts the [CairoPie] input as a zip file and returns it as [Bytes].
    async fn cairo_pie_to_zip_bytes(&self, cairo_pie: CairoPie) -> Result<Bytes> {
        let mut cairo_pie_zipfile = NamedTempFile::new()?;
        cairo_pie.write_zip_file(cairo_pie_zipfile.path())?;
        drop(cairo_pie); // Drop cairo_pie to release the memory
        let cairo_pie_zip_bytes = self.tempfile_to_bytes(&mut cairo_pie_zipfile)?;
        cairo_pie_zipfile.close()?;
        Ok(cairo_pie_zip_bytes)
    }

    /// Converts a [NamedTempFile] to [Bytes].
    /// This function reads the file in chunks and appends them to the buffer.
    /// This is useful when the file is too large to be read in one go.
    fn tempfile_to_bytes(&self, tmp_file: &mut NamedTempFile) -> Result<Bytes> {
        let mut buffer = Vec::new();
        tmp_file.as_file_mut().read_to_end(&mut buffer)?;
        Ok(Bytes::from(buffer))
    }
}
