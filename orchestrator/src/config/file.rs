use anyhow::{Context, Result};
use std::path::Path;

use super::env_interpolation::interpolate_yaml_content;
use super::networks::NetworkRegistry;
use super::{OrchestratorConfig, OrchestratorConfigVersioned};

/// Load configuration from a YAML file with environment variable interpolation
pub fn load_config_file(path: &Path) -> Result<OrchestratorConfig> {
    // Read file content
    let content =
        std::fs::read_to_string(path).with_context(|| format!("Failed to read config file: {}", path.display()))?;

    // Interpolate environment variables
    let interpolated = interpolate_yaml_content(&content).context("Failed to interpolate environment variables")?;

    // Parse YAML with version support
    let versioned =
        OrchestratorConfigVersioned::from_yaml_str(&interpolated).context("Failed to parse configuration file")?;

    // Convert to canonical format
    let mut config = versioned.into_canonical();

    // Register custom networks from config
    let mut registry = NetworkRegistry::builtin();
    registry.register_custom_networks(config.networks.clone());

    // Resolve RPC URLs (DA and Prover can reuse settlement RPC)
    resolve_rpc_urls(&mut config)?;

    // Validate configuration
    validate_config(&config, &registry)?;

    Ok(config)
}

/// Resolve RPC URLs for DA and Prover (can reuse settlement RPC if not specified)
pub(crate) fn resolve_rpc_urls(config: &mut OrchestratorConfig) -> Result<()> {
    let settlement_network = config.settlement.network.clone();
    let settlement_rpc = config.settlement.rpc_url.clone();

    // Resolve DA RPC
    config.data_availability.resolve_rpc_url(&settlement_network, &settlement_rpc);

    if config.data_availability.rpc_url.is_none() {
        anyhow::bail!(
            "DA network '{}' differs from settlement network '{}', but no RPC URL specified",
            config.data_availability.network,
            settlement_network
        );
    }

    // Resolve Prover network and RPC
    config.prover.resolve_network_and_rpc(&settlement_network, &settlement_rpc);

    Ok(())
}

/// Basic validation of configuration
fn validate_config(config: &OrchestratorConfig, registry: &NetworkRegistry) -> Result<()> {
    // Validate settlement network
    config.settlement.validate(registry).context("Settlement configuration validation failed")?;

    // Validate DA network exists
    if !registry.contains(&config.data_availability.network) {
        // Check if it's a custom network
        if !config.networks.iter().any(|n| n.name == config.data_availability.network) {
            anyhow::bail!(
                "Unknown DA network '{}'. Add it to the networks list or built-in registry.",
                config.data_availability.network
            );
        }
    }

    // Validate prover network if specified
    if let Some(ref network) = config.prover.network {
        if !registry.contains(network) && !config.networks.iter().any(|n| &n.name == network) {
            anyhow::bail!("Unknown prover network '{}'. Add it to the networks list or built-in registry.", network);
        }
    }

    // Validate prover config based on type
    match config.prover.prover_type {
        super::types::ProverType::Atlantic => {
            if config.prover.atlantic.is_none() {
                anyhow::bail!("Prover type is 'atlantic' but atlantic config is missing");
            }
        }
        super::types::ProverType::Sharp => {
            if config.prover.sharp.is_none() {
                anyhow::bail!("Prover type is 'sharp' but sharp config is missing");
            }
        }
    }

    // Validate database config
    match config.database.db_type {
        super::types::DatabaseType::MongoDB => {
            if config.database.mongodb.is_none() {
                anyhow::bail!("Database type is 'mongodb' but mongodb config is missing");
            }
        }
    }

    // Validate cloud config
    match config.cloud.provider {
        super::types::CloudProvider::AWS => {
            if config.cloud.aws.is_none() {
                anyhow::bail!("Cloud provider is 'aws' but aws config is missing");
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_load_minimal_config() {
        let yaml = r#"
config_version: "1"

deployment:
  name: test
  layer: l2
  environment: dev

madara:
  rpc_url: "http://localhost:9944"
  version: "0.14.0"

settlement:
  network: ethereum-sepolia
  rpc_url: "http://localhost:8545"
  ethereum:
    private_key: "0x123"
    core_contract_address: "0xabc"
    operator_address: "0xdef"

data_availability:
  network: ethereum-sepolia

prover:
  type: atlantic
  layout:
    snos: all_cairo
    prover: dynamic
  atlantic:
    api_key: "test_key"
    service_url: "https://atlantic.example.com"
    verifier_contract_address: "0x456"
    atlantic_network: TESTNET

snos:
  rpc_url: "http://localhost:9944"

batching:
  max_batch_time_seconds: 600
  max_batch_size: 10000

service:
  min_block_to_process: 0

job_policies:
  snos_execution:
    max_process_attempts: 1
    max_verification_attempts: 1
    verification_polling_delay_seconds: 1
  proving:
    max_process_attempts: 1
    max_verification_attempts: 300
    verification_polling_delay_seconds: 30
  da_submission:
    max_process_attempts: 3
    max_verification_attempts: 10
    verification_polling_delay_seconds: 30
  state_update:
    max_process_attempts: 1
    max_verification_attempts: 20
    verification_polling_delay_seconds: 30
  aggregator:
    max_process_attempts: 1
    max_verification_attempts: 300
    verification_polling_delay_seconds: 30
  proof_registration:
    max_process_attempts: 2
    max_verification_attempts: 300
    verification_polling_delay_seconds: 300

cloud:
  provider: aws
  aws:
    region: us-east-1
    prefix: test
    storage:
      bucket_name: test-bucket
    queue:
      name_pattern: "test-{job_type}"
    alerts:
      topic_name: test-alerts
    event_bridge:
      type: schedule
      interval_seconds: 60

database:
  type: mongodb
  mongodb:
    connection_url: "mongodb://localhost:27017"
    database_name: test

server:
  host: "0.0.0.0"
  port: 3000

observability:
  otel:
    enabled: false
    service_name: orchestrator

operational:
  store_audit_artifacts: false
  graceful_shutdown_timeout_seconds: 120
  mock_atlantic_server: false
"#;

        let mut file = NamedTempFile::new().unwrap();
        file.write_all(yaml.as_bytes()).unwrap();

        let config = load_config_file(file.path());
        assert!(config.is_ok(), "Config loading failed: {:?}", config.err());

        let config = config.unwrap();
        assert_eq!(config.deployment.name, "test");
        assert_eq!(config.settlement.network, "ethereum-sepolia");
    }
}
