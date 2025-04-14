use crate::core::config::Config;
use crate::error::job::snos::SnosError;
use crate::error::job::JobError;
use crate::error::other::OtherError;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{JobMetadata, JobSpecificMetadata, SnosMetadata};
use crate::types::jobs::status::JobVerificationStatus;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::utils::helpers::JobProcessingState;
use crate::utils::COMPILED_OS;
use crate::worker::event_handler::jobs::JobHandlerTrait;
use crate::worker::utils::fact_info::get_fact_info;
use crate::StorageClient;
use async_trait::async_trait;
use bytes::Bytes;
use cairo_vm::types::layout_name::LayoutName;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use cairo_vm::Felt252;
use color_eyre::eyre::eyre;
use color_eyre::Result;
use prove_block::prove_block;
use starknet_os::io::output::StarknetOsOutput;
use std::io::Read;
use std::sync::Arc;
use tempfile::NamedTempFile;

pub struct SnosJobHandler;

#[async_trait]
impl JobHandlerTrait for SnosJobHandler {
    #[tracing::instrument(fields(category = "snos"), skip(self, metadata), ret, err)]
    async fn create_job(&self, internal_id: String, metadata: JobMetadata) -> Result<JobItem, JobError> {
        tracing::info!(
            log_type = "starting",
            category = "snos",
            function_type = "create_job",
            block_no = %internal_id,
            "SNOS job creation started."
        );
        let job_item = JobItem::create(internal_id.clone(), JobType::SnosRun, JobStatus::Created, metadata);
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
        let snos_metadata: SnosMetadata = job.metadata.specific.clone().try_into()?;

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

    fn max_process_attempts(&self) -> u64 {
        1
    }

    fn max_verification_attempts(&self) -> u64 {
        1
    }

    fn verification_polling_delay_seconds(&self) -> u64 {
        1
    }

    fn job_processing_lock(&self, config: Arc<Config>) -> Option<Arc<JobProcessingState>> {
        Some(config.processing_locks().snos_job_processing_lock.clone())
    }
}

impl SnosJobHandler {
    /// Stores the [CairoPie] and the [StarknetOsOutput] in the Data Storage.
    /// The paths will be:
    ///     - [block_number]/cairo_pie.zip
    ///     - [block_number]/snos_output.json
    async fn store(
        &self,
        internal_id: String,
        data_storage: &dyn StorageClient,
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
