use crate::core::config::Config;
use crate::core::StorageClient;
use crate::error::job::fact::FactError;
use crate::error::job::snos::SnosError;
use crate::error::job::JobError;
use crate::error::other::OtherError;
use crate::types::constant::BYTE_CHUNK_SIZE;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{JobMetadata, JobSpecificMetadata, SnosMetadata};
use crate::types::jobs::status::JobVerificationStatus;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::utils::COMPILED_OS;
use crate::worker::event_handler::jobs::JobHandlerTrait;
use crate::worker::utils::fact_info::{build_on_chain_data, get_fact_info, get_fact_l2, get_program_output};
use async_trait::async_trait;
use bytes::Bytes;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use cairo_vm::Felt252;
use color_eyre::eyre::eyre;
use color_eyre::Result;
use orchestrator_utils::layer::Layer;
use prove_block::prove_block;
use starknet_os::io::output::StarknetOsOutput;
use std::sync::Arc;
use tempfile::NamedTempFile;
use tracing::{debug, error, info, trace, warn};

pub struct SnosJobHandler;

#[async_trait]
impl JobHandlerTrait for SnosJobHandler {
    #[tracing::instrument(fields(category = "snos"), skip(self, metadata), ret, err)]
    async fn create_job(&self, internal_id: String, metadata: JobMetadata) -> Result<JobItem, JobError> {
        info!(
            log_type = "starting",
            category = "snos",
            function_type = "create_job",
            block_no = %internal_id,
            "SNOS job creation started."
        );
        let job_item = JobItem::create(internal_id.clone(), JobType::SnosRun, JobStatus::Created, metadata);
        info!(
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
        info!(
            log_type = "starting",
            category = "snos",
            function_type = "process_job",
            job_id = ?job.id,
            block_no = %internal_id,
            "SNOS job processing started."
        );
        debug!(job_id = %job.internal_id, "Processing SNOS job");
        // Get SNOS metadata
        let snos_metadata: SnosMetadata = job.metadata.specific.clone().try_into().inspect_err(|e| {
            error!(job_id = %job.internal_id, error = %e, "Failed to convert metadata to SnosMetadata");
        })?;

        debug!("SNOS metadata retrieved {:?}", snos_metadata);

        // Get block number from metadata
        let block_number = snos_metadata.block_number;
        debug!(job_id = %job.internal_id, block_number = %block_number, "Retrieved block number from metadata");

        let snos_url = config.snos_config().rpc_for_snos.to_string();
        let snos_url = snos_url.trim_end_matches('/');
        debug!(job_id = %job.internal_id, "Calling prove_block function");

        let (cairo_pie, snos_output) =
            prove_block(COMPILED_OS, block_number, snos_url, config.params.snos_layout_name, snos_metadata.full_output)
                .await
                .map_err(|e| {
                    error!(job_id = %job.internal_id, error = %e, "SNOS execution failed");
                    SnosError::SnosExecutionError { internal_id: job.internal_id.clone(), message: e.to_string() }
                })?;
        debug!(job_id = %job.internal_id, "prove_block function completed successfully");

        // We use Layer to determine if we use CallData or Blob for settlement
        // On L1 settlement we have blob-based DA, while on L2 we have CallData based DA
        let (fact_hash, program_output) = match config.layer() {
            Layer::L2 => {
                debug!(job_id = %job.internal_id, "Using blobs for settlement layer");
                // Get the program output from CairoPie
                let fact_info = get_fact_info(&cairo_pie, None, false)?;
                (fact_info.fact, fact_info.program_output)
            }
            Layer::L3 => {
                debug!(job_id = %job.internal_id, "Using CallData for settlement layer");
                // Get the program output from CairoPie
                let fact_hash = get_fact_l2(&cairo_pie, None).map_err(|e| {
                    error!(job_id = %job.internal_id, error = %e, "Failed to get fact hash");
                    JobError::FactError(FactError::L2FactCompute)
                })?;
                let program_output = get_program_output(&cairo_pie, false).map_err(|e| {
                    error!(job_id = %job.internal_id, error = %e, "Failed to get program output");
                    JobError::FactError(FactError::ProgramOutputCompute)
                })?;
                (fact_hash, program_output)
            }
        };

        debug!(job_id = %job.internal_id, "Fact info calculated successfully");

        // Update the metadata with new paths and fact info
        if let JobSpecificMetadata::Snos(metadata) = &mut job.metadata.specific {
            metadata.snos_fact = Some(fact_hash.to_string());
            metadata.snos_n_steps = Some(cairo_pie.execution_resources.n_steps);
        }

        debug!(job_id = %job.internal_id, "Storing SNOS outputs");
        match config.layer() {
            Layer::L2 => {
                // Store the Cairo Pie path
                self.store(
                    internal_id.clone(),
                    config.storage(),
                    &snos_metadata,
                    cairo_pie,
                    snos_output,
                    program_output,
                )
                .await?;
            }
            Layer::L3 => {
                // Store the on-chain data path
                self.store_l2(
                    internal_id.clone(),
                    config.storage(),
                    &snos_metadata,
                    cairo_pie,
                    snos_output,
                    program_output,
                )
                .await?;
            }
        }

        info!(
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
        info!(log_type = "starting", category = "snos", function_type = "verify_job", job_id = %job.id,  block_no = %internal_id, "SNOS job verification started.");
        // No need for verification as of now. If we later on decide to outsource SNOS run
        // to another service, verify_job can be used to poll on the status of the job
        info!(log_type = "completed", category = "snos", function_type = "verify_job", job_id = %job.id,  block_no = %internal_id, "SNOS job verification completed.");
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

impl SnosJobHandler {
    /// Stores the [CairoPie] and the [StarknetOsOutput] and [OnChainData] in the Data Storage.
    /// The paths will be:
    ///     - [block_number]/cairo_pie.zip
    ///     - [block_number]/snos_output.json
    ///     - [block_number]/onchain_data.json
    ///     - [block_number]/program_output.json
    async fn store_l2(
        &self,
        internal_id: String,
        data_storage: &dyn StorageClient,
        snos_metadata: &SnosMetadata,
        cairo_pie: CairoPie,
        snos_output: StarknetOsOutput,
        program_output: Vec<Felt252>,
    ) -> Result<(), SnosError> {
        let on_chain_data = build_on_chain_data(&cairo_pie)
            .map_err(|_e| SnosError::FactCalculationError(FactError::OnChainDataCompute))?;

        self.store(internal_id.clone(), data_storage, snos_metadata, cairo_pie, snos_output, program_output).await?;
        let on_chain_data_key = snos_metadata
            .on_chain_data_path
            .as_ref()
            .ok_or_else(|| SnosError::Other(OtherError(eyre!("OnChain data path is not found"))))?;

        let on_chain_data_vec = serde_json::to_vec(&on_chain_data).map_err(|e| {
            SnosError::OnChainDataUnserializable { internal_id: internal_id.to_string(), message: e.to_string() }
        })?;
        data_storage.put_data(on_chain_data_vec.into(), on_chain_data_key).await.map_err(|e| {
            SnosError::OnChainDataUnstorable { internal_id: internal_id.to_string(), message: e.to_string() }
        })?;
        Ok(())
    }
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
        cairo_pie.write_zip_file(cairo_pie_zipfile.path(), true)?;
        drop(cairo_pie); // Drop cairo_pie to release the memory
        let cairo_pie_zip_bytes = self.tempfile_to_bytes_streaming(&mut cairo_pie_zipfile).await?;
        cairo_pie_zipfile.close()?;
        Ok(cairo_pie_zip_bytes)
    }

    /// Converts a [NamedTempFile] to [Bytes].
    /// This function reads the file in chunks and appends them to the buffer.
    /// This is useful when the file is too large to be read in one go.
    async fn tempfile_to_bytes_streaming(&self, tmp_file: &mut NamedTempFile) -> Result<Bytes> {
        use tokio::io::AsyncReadExt;

        let file_size = tmp_file.as_file().metadata()?.len() as usize;
        let mut buffer = Vec::with_capacity(file_size);

        let mut chunk = vec![0; BYTE_CHUNK_SIZE];

        let mut file = tokio::fs::File::from_std(tmp_file.as_file().try_clone()?);

        while let Ok(n) = file.read(&mut chunk).await {
            if n == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..n]);
        }

        Ok(Bytes::from(buffer))
    }
}
