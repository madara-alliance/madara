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
use crate::worker::event_handler::jobs::JobHandlerTrait;
use crate::worker::utils::fact_info::{build_on_chain_data, get_fact_info, get_fact_l2, get_program_output};
use async_trait::async_trait;
use bytes::Bytes;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use cairo_vm::Felt252;
use color_eyre::eyre::eyre;
use color_eyre::Result;
use generate_pie::generate_pie;
use generate_pie::types::chain_config::ChainConfig;
use generate_pie::types::os_hints::OsHintsConfiguration;
use generate_pie::types::pie::{PieGenerationInput, PieGenerationResult};
use orchestrator_utils::layer::Layer;
use starknet::providers::jsonrpc::{HttpTransport, JsonRpcClient};
use starknet::providers::Provider;
use starknet::providers::Url;
use starknet_api::core::{ChainId, ContractAddress};
use starknet_core::types::Felt;

use std::sync::Arc;
use tempfile::NamedTempFile;
use tracing::{debug, error, info};

pub struct SnosJobHandler;

#[async_trait]
pub trait ChainConfigFromExt {
    async fn get_chain_config(
        rpc_url: &str,
        layer: &Layer,
        strk_fee_token_address: &str,
        eth_fee_token_address: &str,
    ) -> Result<ChainConfig>;
}

#[async_trait]
impl ChainConfigFromExt for ChainConfig {
    async fn get_chain_config(
        rpc_url: &str,
        layer: &Layer,
        strk_fee_token_address: &str,
        eth_fee_token_address: &str,
    ) -> Result<ChainConfig> {
        let rpc_url = Url::parse(rpc_url)?;
        let provider = JsonRpcClient::new(HttpTransport::new(rpc_url));
        let chain_id_in_hex = provider.chain_id().await?.to_fixed_hex_string();

        let chain_id_decoded = String::from_utf8(hex::decode(chain_id_in_hex.trim_start_matches("0x"))?)?;
        let chain_id = ChainId::Other(chain_id_decoded);

        Ok(ChainConfig {
            chain_id,
            strk_fee_token_address: ContractAddress::try_from(Felt::from_hex_unchecked(strk_fee_token_address))?,
            eth_fee_token_address: ContractAddress::try_from(Felt::from_hex_unchecked(eth_fee_token_address))?,
            is_l3: layer.is_l3(),
        })
    }
}

trait OsHintsConfigurationFromLayer {
    fn with_layer(layer: Layer) -> OsHintsConfiguration;
}

impl OsHintsConfigurationFromLayer for OsHintsConfiguration {
    fn with_layer(layer: Layer) -> OsHintsConfiguration {
        match layer {
            Layer::L2 => OsHintsConfiguration { debug_mode: true, full_output: true, use_kzg_da: false },
            Layer::L3 => OsHintsConfiguration { debug_mode: true, full_output: false, use_kzg_da: false },
        }
    }
}

#[async_trait]
impl JobHandlerTrait for SnosJobHandler {
    async fn create_job(&self, internal_id: String, metadata: JobMetadata) -> Result<JobItem, JobError> {
        info!(log_type = "starting", "SNOS job creation started.");
        let job_item = JobItem::create(internal_id.clone(), JobType::SnosRun, JobStatus::Created, metadata);
        info!(log_type = "completed", "SNOS job creation completed.");
        Ok(job_item)
    }

    async fn process_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<String, JobError> {
        let internal_id = job.internal_id.clone();
        info!(log_type = "starting", "SNOS job processing started.");
        debug!("Processing SNOS job");
        // Get SNOS metadata
        let snos_metadata: SnosMetadata = job.metadata.specific.clone().try_into().inspect_err(|e| {
            error!(error = %e, "Failed to convert metadata to SnosMetadata");
        })?;

        debug!("SNOS metadata retrieved {:?}", snos_metadata);

        // Get block number from metadata (using start_block as the primary block for processing)
        let start_block_number = snos_metadata.start_block;
        let end_block_number = snos_metadata.end_block;
        debug!(start_block = %snos_metadata.start_block, end_block = %snos_metadata.end_block, num_blocks = %snos_metadata.num_blocks, "Retrieved batch information from metadata");

        let snos_url = config.snos_config().rpc_for_snos.to_string();
        let snos_url = snos_url.trim_end_matches('/');
        debug!("Calling generate_pie function");

        let input = PieGenerationInput {
            rpc_url: snos_url.to_string(),
            blocks: (start_block_number..=end_block_number).collect(),
            // chain_config: ChainConfig::default(),
            chain_config: ChainConfig::get_chain_config(
                snos_url,
                config.layer(),
                &config.params.snos_config.strk_fee_token_address,
                &config.params.snos_config.eth_fee_token_address,
            )
            .await
            .map_err(|e| JobError::Other(OtherError(eyre!("Failed to get chain config: {}", e))))?,
            os_hints_config: OsHintsConfiguration::with_layer(config.layer().clone()),
            output_path: None, // No file output
            layout: config.params.snos_layout_name,
        };

        let snos_output: PieGenerationResult = generate_pie(input).await.map_err(|e| {
            error!(error = %e, "SNOS execution failed");
            SnosError::SnosExecutionError { internal_id: job.internal_id.clone(), message: e.to_string() }
        })?;
        debug!("generate_pie function completed successfully");

        let cairo_pie = snos_output.output.cairo_pie;

        // TODO: currently we are getting the Vec<Felt> but ideally we should get a struct, fix it once it available upstream
        //       maybe we can add our own struct meanwhile in the snos code? something to think about!!!
        let os_output = snos_output.output.raw_os_output;
        // We use KZG_DA flag in order to determine whether we are using L1 or L2 as
        // settlement layer. On L1 settlement we have blob based DA, while on L2 we have
        // calldata based DA.
        // So in case of KZG flag == 0 :
        //      we calculate the l2 fact
        // And in case of KZG flag == 1 :
        //      we calculate the fact info
        let (fact_hash, program_output) = if os_output.get(8) == Some(&Felt::ZERO) {
            debug!("Using calldata for settlement layer");
            // Get the program output from CairoPie
            let fact_hash = get_fact_l2(&cairo_pie, None).map_err(|e| {
                error!(error = %e, "Failed to get fact hash");
                JobError::FactError(FactError::L2FactCompute)
            })?;
            let program_output = get_program_output(&cairo_pie, false).map_err(|e| {
                error!(error = %e, "Failed to get program output");
                JobError::FactError(FactError::ProgramOutputCompute)
            })?;
            (fact_hash, program_output)
        } else if os_output.get(8) == Some(&Felt::ONE) {
            debug!("Using blobs for settlement layer");
            // Get the program output from CairoPie
            let fact_info = get_fact_info(&cairo_pie, None, false)?;
            (fact_info.fact, fact_info.program_output)
        } else {
            error!("Invalid KZG flag");
            return Err(JobError::from(SnosError::UnsupportedKZGFlag));
        };

        debug!("Fact info calculated successfully");

        // Update the metadata with new paths and fact info
        if let JobSpecificMetadata::Snos(metadata) = &mut job.metadata.specific {
            metadata.snos_fact = Some(fact_hash.to_string());
            metadata.snos_n_steps = Some(cairo_pie.execution_resources.n_steps);
        }

        debug!("Storing SNOS outputs");
        if config.layer() == &Layer::L3 {
            // Store the on-chain data path
            self.store_l2(internal_id.clone(), config.storage(), &snos_metadata, cairo_pie, os_output, program_output)
                .await?;
        } else if config.layer() == &Layer::L2 {
            // Store the Cairo Pie path
            self.store(internal_id.clone(), config.storage(), &snos_metadata, cairo_pie, os_output, program_output)
                .await?;
        }

        info!(log_type = "completed", "SNOS job processed successfully.");

        Ok(snos_metadata.snos_batch_index.to_string())
    }

    async fn verify_job(&self, _config: Arc<Config>, _job: &mut JobItem) -> Result<JobVerificationStatus, JobError> {
        info!(log_type = "starting", "SNOS job verification started.");
        // No need for verification as of now. If we later on decide to outsource SNOS run
        // to another service, verify_job can be used to poll on the status of the job
        info!(log_type = "completed", "SNOS job verification completed.");
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
        snos_output: Vec<Felt>,
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
        snos_output: Vec<Felt>,
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
