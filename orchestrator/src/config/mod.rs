pub mod builder;
pub mod env_interpolation;
pub mod file;
pub mod networks;
pub mod presets;
pub mod types;
pub mod validation;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub use builder::{load_config_from_run_cmd, ConfigBuilder};
pub use types::*;

/// Versioned configuration wrapper
/// This allows us to evolve the config format over time while maintaining backward compatibility
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "config_version")]
pub enum OrchestratorConfigVersioned {
    #[serde(rename = "1")]
    V1(OrchestratorConfigV1),
}

impl OrchestratorConfigVersioned {
    /// Load configuration from a YAML file
    pub fn from_yaml_file(path: &Path) -> Result<Self> {
        let content =
            std::fs::read_to_string(path).with_context(|| format!("Failed to read config file: {}", path.display()))?;

        Self::from_yaml_str(&content)
    }

    /// Load configuration from a YAML string
    pub fn from_yaml_str(content: &str) -> Result<Self> {
        // First check if config_version field exists
        let yaml_value: serde_yaml::Value = serde_yaml::from_str(content).context("Failed to parse YAML")?;

        if !yaml_value.get("config_version").is_some() {
            anyhow::bail!(
                "Missing required field 'config_version' in config file. \
                 Current supported version: 1"
            );
        }

        // Deserialize with version dispatch
        let versioned: OrchestratorConfigVersioned =
            serde_yaml::from_str(content).context("Failed to deserialize config")?;

        Ok(versioned)
    }

    /// Convert to the canonical (latest) config format
    pub fn into_canonical(self) -> OrchestratorConfig {
        match self {
            OrchestratorConfigVersioned::V1(v1) => v1.into(),
        }
    }
}

/// Canonical configuration (always latest version internally)
pub type OrchestratorConfig = OrchestratorConfigV1;

/// Version 1 of the orchestrator configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorConfigV1 {
    pub deployment: DeploymentConfig,
    pub madara: MadaraConfig,
    pub settlement: SettlementConfig,
    pub data_availability: DataAvailabilityConfig,
    pub prover: ProverConfig,
    pub snos: SNOSConfig,
    pub batching: BatchingConfig,
    pub service: ServiceConfig,
    pub job_policies: JobPoliciesConfig,
    pub cloud: CloudProviderConfig,
    pub database: DatabaseConfig,
    pub server: ServerConfig,
    pub observability: ObservabilityConfig,
    pub operational: OperationalConfig,

    /// Custom networks defined in this config file
    #[serde(default)]
    pub networks: Vec<NetworkInfo>,
}

// No need for From impl since they're the same type (type alias)

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_missing_config_version() {
        let yaml = r#"
deployment:
  name: test
"#;
        let result = OrchestratorConfigVersioned::from_yaml_str(yaml);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("config_version"));
    }

    #[test]
    fn test_invalid_config_version() {
        let yaml = r#"
config_version: "999"
deployment:
  name: test
"#;
        let result = OrchestratorConfigVersioned::from_yaml_str(yaml);
        assert!(result.is_err());
    }
}
