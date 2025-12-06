use anyhow::{Context, Result};

use super::networks::NetworkRegistry;
use super::OrchestratorConfig;

pub trait Validate {
    fn validate(&self, registry: &NetworkRegistry) -> Result<()>;
}

impl Validate for OrchestratorConfig {
    fn validate(&self, registry: &NetworkRegistry) -> Result<()> {
        // Validate all sub-configurations
        validate_structure(self)?;
        validate_ranges(self)?;
        validate_consistency(self, registry)?;

        Ok(())
    }
}

fn validate_structure(config: &OrchestratorConfig) -> Result<()> {
    // Check required URLs are valid
    let _ = config.madara.rpc_url.as_str();
    let _ = config.settlement.rpc_url.as_str();

    // Check version is supported
    if !config.madara.version.is_supported() {
        anyhow::bail!(
            "Unsupported Madara version: {}. Supported versions: {:?}",
            config.madara.version,
            super::types::StarknetVersion::supported()
        );
    }

    Ok(())
}

fn validate_ranges(config: &OrchestratorConfig) -> Result<()> {
    // Batching config
    if config.batching.max_batch_size == 0 {
        anyhow::bail!("batching.max_batch_size must be > 0");
    }

    if config.batching.max_batch_time_seconds == 0 {
        anyhow::bail!("batching.max_batch_time_seconds must be > 0");
    }

    if config.batching.max_num_blobs == 0 || config.batching.max_num_blobs > 6 {
        anyhow::bail!("batching.max_num_blobs must be between 1 and 6");
    }

    // Job policies
    validate_job_policy("snos_execution", &config.job_policies.snos_execution)?;
    validate_job_policy("proving", &config.job_policies.proving)?;
    validate_job_policy("da_submission", &config.job_policies.da_submission)?;
    validate_job_policy("state_update", &config.job_policies.state_update)?;
    validate_job_policy("aggregator", &config.job_policies.aggregator)?;
    validate_job_policy("proof_registration", &config.job_policies.proof_registration)?;

    Ok(())
}

fn validate_job_policy(name: &str, policy: &super::types::JobPolicy) -> Result<()> {
    if policy.max_process_attempts == 0 {
        anyhow::bail!("job_policies.{}.max_process_attempts must be > 0", name);
    }

    if policy.max_verification_attempts == 0 {
        anyhow::bail!("job_policies.{}.max_verification_attempts must be > 0", name);
    }

    if policy.verification_polling_delay_seconds == 0 {
        anyhow::bail!("job_policies.{}.verification_polling_delay_seconds must be > 0", name);
    }

    Ok(())
}

fn validate_consistency(config: &OrchestratorConfig, registry: &NetworkRegistry) -> Result<()> {
    // Validate settlement network
    config.settlement.validate(registry).context("Settlement configuration validation failed")?;

    // Validate DA network
    if !registry.contains(&config.data_availability.network) {
        if !config.networks.iter().any(|n| n.name == config.data_availability.network) {
            anyhow::bail!(
                "Unknown DA network '{}'. Add it to the networks list or built-in registry.",
                config.data_availability.network
            );
        }
    }

    // Validate DA RPC is set
    if config.data_availability.rpc_url.is_none() {
        anyhow::bail!("Data availability RPC URL must be set (either explicitly or resolved from settlement)");
    }

    // Validate prover network
    if let Some(ref network) = config.prover.network {
        if !registry.contains(network) && !config.networks.iter().any(|n| &n.name == network) {
            anyhow::bail!("Unknown prover network '{}'. Add it to the networks list or built-in registry.", network);
        }
    }

    // Validate prover RPC is set
    if config.prover.rpc_url.is_none() {
        anyhow::bail!("Prover RPC URL must be set (either explicitly or resolved from settlement)");
    }

    Ok(())
}
