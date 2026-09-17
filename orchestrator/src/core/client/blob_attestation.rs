//! Trusted configuration for the orchestrator's dedicated attestation API.
use color_eyre::eyre::{ensure, Result, WrapErr};
use kzg_attestation_protocol::Policy;
use serde::Deserialize;
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlobAttestationConfig {
    pub listen: SocketAddr,
    pub auth_token_file: PathBuf,
    pub policy: Policy,
}

impl BlobAttestationConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let config: Self = serde_json::from_slice(&std::fs::read(path).wrap_err("Cannot read attestation config")?)?;
        config.policy.validate()?;
        config.auth_token()?;
        Ok(config)
    }

    pub fn auth_token(&self) -> Result<String> {
        let token = std::fs::read_to_string(&self.auth_token_file).wrap_err("Cannot read attestation API token")?;
        let token = token.trim();
        ensure!(
            token.len() >= 32 && token.is_ascii() && !token.contains(char::is_whitespace),
            "Attestation API token must contain at least 32 non-whitespace ASCII characters"
        );
        Ok(token.to_owned())
    }
}
