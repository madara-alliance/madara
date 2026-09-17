//! Publish immutable work, then wait for independently submitted blob checks.
use crate::core::config::Config;
use crate::error::{job::JobError, other::OtherError};
use crate::types::jobs::{
    job_item::JobItem,
    metadata::{JobMetadata, JobSpecificMetadata, SignatureCollectionMetadata, SignatureReceipt},
    status::JobVerificationStatus,
    types::{JobStatus, JobType},
};
use crate::worker::{event_handler::jobs::JobHandlerTrait, utils::fetch_program_output};
use async_trait::async_trait;
use color_eyre::eyre::{ensure, eyre, Result};
use kzg_attestation_protocol::{recover_signer, signing_digest, validate_output, verify_certificate, Certificate};
use std::{collections::BTreeMap, sync::Arc};

pub struct SignatureCollectionJobHandler;

/// Revalidate durable receipts independently of the HTTP submission checks.
pub(crate) fn assemble_certificate(
    job_id: &str,
    metadata: &SignatureCollectionMetadata,
    receipts: Vec<SignatureReceipt>,
) -> Result<Option<Certificate>> {
    validate_output(&metadata.program_output, &metadata.policy)?;
    let digest = signing_digest(&metadata.program_output, &metadata.policy);
    ensure!(metadata.digest == Some(digest), "Published work digest mismatch");
    let mut signatures = BTreeMap::new();
    for receipt in receipts {
        ensure!(receipt.job_id == job_id && receipt.digest == digest, "Receipt belongs to different work");
        let signer = recover_signer(digest, &receipt.signature)?;
        ensure!(signer == receipt.signer && metadata.policy.members.contains(&signer), "Invalid receipt signer");
        signatures.entry(signer).or_insert(receipt.signature);
    }
    if signatures.len() < metadata.policy.threshold {
        return Ok(None);
    }
    let certificate = Certificate {
        digest,
        committee_epoch: metadata.policy.committee_epoch,
        signatures: signatures.into_values().take(metadata.policy.threshold).collect(),
    };
    verify_certificate(&metadata.program_output, &metadata.policy, &certificate)?;
    Ok(Some(certificate))
}

#[async_trait]
impl JobHandlerTrait for SignatureCollectionJobHandler {
    async fn create_job(&self, internal_id: u64, metadata: JobMetadata) -> Result<JobItem, JobError> {
        let _: SignatureCollectionMetadata = metadata.specific.clone().try_into()?;
        Ok(JobItem::create(internal_id, JobType::SignatureCollection, JobStatus::Created, metadata))
    }

    async fn process_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<String, JobError> {
        let mut metadata: SignatureCollectionMetadata = job.metadata.specific.clone().try_into()?;
        let configured = config
            .params
            .blob_attestation
            .as_ref()
            .ok_or_else(|| OtherError(eyre!("Signature collection is disabled")))?;
        if configured.policy != metadata.policy {
            return Err(OtherError(eyre!("Signature job policy differs from configured committee")).into());
        }
        config.settlement_client().validate_blob_attestation_policy(&metadata.policy).await.map_err(OtherError)?;
        let output = fetch_program_output(config, &Some(metadata.aggregator.program_output_path.clone())).await?;
        let output = output.into_iter().map(Into::into).collect::<Vec<_>>();
        validate_output(&output, &metadata.policy).map_err(|e| OtherError(eyre!(e)))?;
        let digest = signing_digest(&output, &metadata.policy);
        if metadata.digest.is_some_and(|published| published != digest) {
            return Err(OtherError(eyre!("Aggregator output changed after work publication")).into());
        }
        metadata.program_output = output;
        metadata.digest = Some(digest);
        // Generic job processing persists this snapshot and PendingVerification together.
        // Only that durable state is visible through the polling API.
        job.metadata.specific = JobSpecificMetadata::SignatureCollection(metadata);
        Ok(digest.to_string())
    }

    async fn verify_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<JobVerificationStatus, JobError> {
        let mut metadata: SignatureCollectionMetadata = job.metadata.specific.clone().try_into()?;
        let digest = metadata.digest.ok_or_else(|| OtherError(eyre!("Signature work has no digest")))?;
        let receipts = config.database().get_signature_receipts(&job.id.to_string(), &digest).await?;
        let Some(certificate) = assemble_certificate(&job.id.to_string(), &metadata, receipts).map_err(OtherError)?
        else {
            return Ok(JobVerificationStatus::Pending);
        };
        config.settlement_client().validate_blob_attestation_policy(&metadata.policy).await.map_err(OtherError)?;
        metadata.certificate = Some(certificate);
        job.metadata.specific = JobSpecificMetadata::SignatureCollection(metadata);
        Ok(JobVerificationStatus::Verified)
    }

    fn max_process_attempts(&self) -> u64 {
        3
    }
    // An offline signer is a waiting condition; it must not time out a valid batch.
    fn max_verification_attempts(&self) -> u64 {
        u64::MAX
    }
    fn verification_polling_delay_seconds(&self) -> u64 {
        15
    }
}
