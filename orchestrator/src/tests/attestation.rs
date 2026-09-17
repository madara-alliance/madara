use crate::core::client::{blob_attestation::BlobAttestationConfig, database::MockDatabaseClient};
use crate::tests::config::TestConfigBuilder;
use crate::types::{
    constant::ORCHESTRATOR_VERSION,
    jobs::{
        job_item::JobItem,
        metadata::{
            AggregatorMetadata, CommonMetadata, JobMetadata, JobSpecificMetadata, SignatureCollectionMetadata,
            SignatureReceipt,
        },
        status::JobVerificationStatus,
        types::{JobStatus, JobType},
    },
};
use crate::worker::event_handler::jobs::{
    signature_collection::{assemble_certificate, SignatureCollectionJobHandler},
    JobHandlerTrait,
};
use alloy::{
    primitives::{Bytes, B256},
    signers::{local::PrivateKeySigner, SignerSync},
};
use kzg_attestation_protocol::{signing_digest, verify_certificate, word, Attestation, Policy, WorkPage};
use orchestrator_settlement_client_interface::MockSettlementClient;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

const TOKEN: &str = "public-attestation-api-test-token-not-a-secret";

#[test]
fn attestation_api_only_is_opt_in_and_requires_committee_configuration() {
    use crate::cli::RunCmd;
    use clap::{CommandFactory, FromArgMatches};

    let parse = |extra: &[&str]| {
        let mut args = vec![
            "orchestrator",
            "--aws",
            "--aws-s3",
            "--aws-sqs",
            "--aws-sns",
            "--queue-identifier",
            "test-queue",
            "--settle-on-ethereum",
            "--da-on-ethereum",
            "--ethereum-da-rpc-url",
            "http://localhost:8545",
            "--madara-rpc-url",
            "http://localhost:9944",
            "--rpc-for-snos",
            "http://localhost:9545",
            "--max-batch-time-seconds",
            "1800",
            "--prover",
            "mock",
        ];
        args.extend_from_slice(extra);
        let matches = RunCmd::command().mut_args(|arg| arg.env(None::<&str>)).try_get_matches_from(args)?;
        RunCmd::from_arg_matches(&matches)
    };
    assert!(!parse(&[]).unwrap().attestation_api_only);
    let error = parse(&["--attestation-api-only"]).unwrap_err();
    assert_eq!(error.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    assert!(error.to_string().contains("--blob-attestation-config"));
    assert!(
        parse(&["--attestation-api-only", "--blob-attestation-config", "committee.json"]).unwrap().attestation_api_only
    );
}

fn fixture() -> (JobItem, Vec<Attestation>) {
    let value: serde_json::Value = serde_json::from_str(include_str!("attestation-fixture.json")).unwrap();
    let mut policy: Policy = serde_json::from_value(value["policy"].clone()).unwrap();
    let output: Vec<B256> = serde_json::from_value(value["program_output"].clone()).unwrap();
    let signers = [1, 2].map(|key| PrivateKeySigner::from_bytes(&word(key)).unwrap());
    policy.members = signers.iter().map(|signer| signer.address()).collect();
    policy.threshold = 2;
    let digest = signing_digest(&output, &policy);
    let attestations = signers
        .iter()
        .map(|signer| Attestation {
            digest,
            committee_epoch: policy.committee_epoch,
            signer: signer.address(),
            signature: Bytes::copy_from_slice(&signer.sign_hash_sync(&digest).unwrap().as_bytes()),
        })
        .collect();
    let job = JobItem::create(
        1,
        JobType::SignatureCollection,
        JobStatus::PendingVerification,
        JobMetadata {
            common: CommonMetadata { orchestrator_version: ORCHESTRATOR_VERSION.into(), ..Default::default() },
            specific: JobSpecificMetadata::SignatureCollection(SignatureCollectionMetadata {
                aggregator: Box::new(AggregatorMetadata::default()),
                policy,
                program_output: output,
                digest: Some(digest),
                certificate: None,
            }),
        },
    );
    (job, attestations)
}
fn receipt(job: &JobItem, attestation: &Attestation) -> SignatureReceipt {
    SignatureReceipt {
        id: format!("{}:{}:{}", job.id, attestation.digest, attestation.signer),
        job_id: job.id.to_string(),
        digest: attestation.digest,
        signer: attestation.signer,
        signature: attestation.signature.clone(),
    }
}

#[test]
fn attestation_receipts_require_distinct_signers_and_exact_published_work() {
    let (job, signatures) = fixture();
    let metadata: SignatureCollectionMetadata = job.metadata.specific.clone().try_into().unwrap();
    let first = receipt(&job, &signatures[0]);
    assert!(assemble_certificate(&job.id.to_string(), &metadata, vec![first.clone(), first.clone()])
        .unwrap()
        .is_none());
    let second = receipt(&job, &signatures[1]);
    let certificate =
        assemble_certificate(&job.id.to_string(), &metadata, vec![second.clone(), first.clone()]).unwrap().unwrap();
    verify_certificate(&metadata.program_output, &metadata.policy, &certificate).unwrap();
    let mut wrong = first.clone();
    wrong.job_id = "different-job".into();
    assert!(assemble_certificate(&job.id.to_string(), &metadata, vec![wrong, second.clone()]).is_err());
    let mut tampered = metadata.clone();
    tampered.program_output[0] = word(100);
    assert!(assemble_certificate(&job.id.to_string(), &tampered, vec![first, second]).is_err());
}

#[tokio::test]
async fn attestation_api_is_isolated_authenticated_and_receipts_unlock_verification() {
    let (job, signatures) = fixture();
    let metadata: SignatureCollectionMetadata = job.metadata.specific.clone().try_into().unwrap();
    let stored = Arc::new(Mutex::new(BTreeMap::<String, SignatureReceipt>::new()));
    let mut db = MockDatabaseClient::new();
    let lookup = job.clone();
    db.expect_get_job_by_id().returning(move |id| Ok((id == lookup.id).then(|| lookup.clone())));
    let offer = job.clone();
    db.expect_get_signature_work().returning(move |_, _, _| Ok(vec![offer.clone()]));
    let reads = stored.clone();
    db.expect_get_signature_receipts().returning(move |id, digest| {
        Ok(reads.lock().unwrap().values().filter(|r| r.job_id == id && r.digest == *digest).cloned().collect())
    });
    let writes = stored.clone();
    db.expect_store_signature_receipt().returning(move |r| {
        writes.lock().unwrap().entry(r.id.clone()).or_insert(r);
        Ok(())
    });
    let mut settlement = MockSettlementClient::new();
    settlement.expect_validate_blob_attestation_policy().times(1).returning(|_| Ok(()));
    let services = TestConfigBuilder::new()
        .configure_database(db.into())
        .configure_settlement_client(settlement.into())
        .build()
        .await;
    let mut config = services.config;
    let token_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(token_file.path(), TOKEN).unwrap();
    Arc::get_mut(&mut config).unwrap().params.blob_attestation = Some(BlobAttestationConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        auth_token_file: token_file.path().into(),
        policy: metadata.policy,
    });
    // API-only startup must not bind or even resolve the regular server address.
    Arc::get_mut(&mut config).unwrap().params.server_config.host = "invalid::address".into();
    let (address, server) = crate::server::setup_attestation_server(config.clone()).await.unwrap();
    let base = format!("http://{address}");
    let client = reqwest::Client::new();
    for path in ["/admin", "/jobs", "/api/v1/jobs", "/blocks", "/batches"] {
        assert_eq!(client.get(format!("{base}{path}")).bearer_auth(TOKEN).send().await.unwrap().status(), 404);
    }
    let work = format!("{base}/v1/attestations/work?signer={}", signatures[0].signer);
    assert_eq!(client.get(&work).send().await.unwrap().status(), 401);
    let page: WorkPage = client.get(&work).bearer_auth(TOKEN).send().await.unwrap().json().await.unwrap();
    assert_eq!(page.jobs.len(), 1);
    let endpoint = format!("{base}/v1/attestations/work/{}/signatures", job.id);
    let mut verification_job = job.clone();
    assert_eq!(
        SignatureCollectionJobHandler.verify_job(config.clone(), &mut verification_job).await.unwrap(),
        JobVerificationStatus::Pending
    );
    // Duplicate POSTs are accepted but only one recovered signer is durable.
    for _ in 0..2 {
        assert_eq!(client.post(&endpoint).bearer_auth(TOKEN).json(&signatures[0]).send().await.unwrap().status(), 202);
    }
    assert_eq!(stored.lock().unwrap().len(), 1);
    assert_eq!(
        SignatureCollectionJobHandler.verify_job(config.clone(), &mut verification_job).await.unwrap(),
        JobVerificationStatus::Pending
    );
    let page: WorkPage = client.get(&work).bearer_auth(TOKEN).send().await.unwrap().json().await.unwrap();
    assert!(page.jobs.is_empty());
    let mut invalid = signatures[1].clone();
    invalid.digest = B256::ZERO;
    assert_eq!(client.post(&endpoint).bearer_auth(TOKEN).json(&invalid).send().await.unwrap().status(), 422);
    assert_eq!(client.post(&endpoint).bearer_auth(TOKEN).json(&signatures[1]).send().await.unwrap().status(), 202);
    assert_eq!(
        SignatureCollectionJobHandler.verify_job(config, &mut verification_job).await.unwrap(),
        JobVerificationStatus::Verified
    );
    let verified: SignatureCollectionMetadata = verification_job.metadata.specific.try_into().unwrap();
    assert!(verified.certificate.is_some());
    server.shutdown().await.unwrap();
}
