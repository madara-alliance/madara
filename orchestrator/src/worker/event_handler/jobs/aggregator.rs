use crate::core::config::Config;
use crate::core::StorageClient;
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

pub struct AggregatorJobHandler;

#[async_trait]
impl JobHandlerTrait for AggregatorJobHandler {
    #[tracing::instrument(fields(category = "aggregator"), skip(self, metadata), ret, err)]
    async fn create_job(&self, internal_id: String, metadata: JobMetadata) -> Result<JobItem, JobError> {
        tracing::info!(
            log_type = "starting",
            category = "aggregator",
            function_type = "create_job",
            block_no = %internal_id,
            "Aggregator job creation started."
        );
        let job_item = JobItem::create(internal_id.clone(), JobType::SnosRun, JobStatus::Created, metadata);
        tracing::info!(
            log_type = "completed",
            category = "aggregator",
            function_type = "create_job",
            block_no = %internal_id,
            "Aggregator job creation completed."
        );
        Ok(job_item)
    }

    #[tracing::instrument(fields(category = "aggregator"), skip(self, config), ret, err)]
    async fn process_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<String, JobError> {
        todo!()
    }

    #[tracing::instrument(fields(category = "aggregator"), skip(self, _config), ret, err)]
    async fn verify_job(&self, _config: Arc<Config>, job: &mut JobItem) -> Result<JobVerificationStatus, JobError> {
        todo!()
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
        todo!()
        // config.processing_locks().snos_job_processing_lock.clone()
    }
}