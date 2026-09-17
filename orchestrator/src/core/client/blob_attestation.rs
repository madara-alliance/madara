//! Optional, independently configured committee client. Never derives trust from the coordinator.
use alloy::primitives::B256;
use color_eyre::eyre::{bail, ensure, Result, WrapErr};
use kzg_attestation_protocol::{validate_request, verify_certificate, AttestationRequest, Certificate, Policy};
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use url::Url;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlobAttestationConfig {
    pub coordinator_url: Url,
    pub auth_token_file: PathBuf,
    pub policy: Policy,
}

impl BlobAttestationConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let config: Self = serde_json::from_slice(&std::fs::read(path).wrap_err("Cannot read attestation config")?)?;
        config.policy.validate()?;
        let url = &config.coordinator_url;
        ensure!(
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "Coordinator must be an HTTP(S) base URL without credentials"
        );
        let token = std::fs::read_to_string(&config.auth_token_file).wrap_err("Cannot read coordinator token")?;
        ensure!(!token.trim().is_empty(), "Coordinator token is empty");
        Ok(config)
    }

    pub async fn attest(&self, output: &[[u8; 32]], blobs: &[Vec<u8>]) -> Result<Certificate> {
        let request = AttestationRequest {
            chain_id: self.policy.chain_id,
            core_address: self.policy.core_address,
            committee_epoch: self.policy.committee_epoch,
            program_output: output.iter().copied().map(B256::from).collect(),
            blobs: blobs.iter().cloned().map(Into::into).collect(),
        };
        validate_request(&request, &self.policy)?;
        let token = std::fs::read_to_string(&self.auth_token_file).wrap_err("Cannot read coordinator token")?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let url = format!("{}/v1/aggregate", self.coordinator_url.as_str().trim_end_matches('/'));
        // Do not include response bodies or bearer credentials in errors/logs.
        let mut response = client.post(url).bearer_auth(token.trim()).json(&request).send().await?;
        ensure!(response.status().is_success(), "Coordinator returned status {}", response.status());
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > 32768 {
                bail!("Coordinator certificate exceeds size limit");
            }
            bytes.extend_from_slice(&chunk);
        }
        let certificate: Certificate = serde_json::from_slice(&bytes)?;
        verify_certificate(&request.program_output, &self.policy, &certificate)?;
        Ok(certificate)
    }
}
