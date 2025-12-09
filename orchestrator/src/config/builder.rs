//! Configuration builder that implements the hierarchy: CLI > ENV > Config File > Defaults

use super::file::load_config_file;
use super::networks::NetworkRegistry;
use super::presets::PresetRegistry;
use super::validation::Validate;
use super::OrchestratorConfig;
use crate::cli::RunCmd;
use anyhow::{Context, Result};
use std::path::Path;
use tracing::{debug, info};

/// Configuration builder that merges config from multiple sources
pub struct ConfigBuilder {
    /// Base config loaded from file or preset
    base_config: Option<OrchestratorConfig>,
    /// Network registry for validation
    registry: NetworkRegistry,
}

impl ConfigBuilder {
    /// Create a new config builder
    pub fn new() -> Self {
        Self { base_config: None, registry: NetworkRegistry::default() }
    }

    /// Load config from a preset name
    pub fn with_preset(mut self, preset_name: &str) -> Result<Self> {
        info!("Loading configuration from preset: {}", preset_name);
        let preset_registry = PresetRegistry::default();
        let config = preset_registry.load_preset(preset_name)?;
        self.base_config = Some(config);
        Ok(self)
    }

    /// Load config from a file path
    pub fn with_config_file(mut self, path: &Path) -> Result<Self> {
        info!("Loading configuration from file: {}", path.display());
        let config = load_config_file(path)?;
        self.base_config = Some(config);
        Ok(self)
    }

    /// Apply CLI overrides on top of the base config
    /// This implements the hierarchy: CLI args > ENV vars > Config file
    pub fn with_cli_overrides(mut self, run_cmd: &RunCmd) -> Result<Self> {
        if self.base_config.is_none() {
            return Err(anyhow::anyhow!(
                "Cannot apply CLI overrides without a base config (use with_preset or with_config_file first)"
            ));
        }

        let config = self.base_config.as_mut().unwrap();
        debug!("Applying CLI overrides to configuration");

        // Apply deployment overrides
        if let Some(layer) = &run_cmd.layer {
            debug!("Overriding deployment.layer from CLI: {:?}", layer);
            // Convert from orchestrator_utils::layer::Layer to config::types::deployment::Layer
            use super::types::deployment::Layer as ConfigLayer;
            use crate::types::Layer as UtilsLayer;
            config.deployment.layer = match layer {
                UtilsLayer::L2 => ConfigLayer::L2,
                UtilsLayer::L3 => ConfigLayer::L3,
            };
        }

        // Apply Madara overrides
        if let Some(rpc_url) = &run_cmd.madara_rpc_url {
            debug!("Overriding madara.rpc_url from CLI");
            config.madara.rpc_url = rpc_url.clone();
        }
        if let Some(version) = &run_cmd.madara_version {
            debug!("Overriding madara.version from CLI: {}", version);
            // Convert from core::config::StarknetVersion to config::types::madara::StarknetVersion
            use super::types::madara::StarknetVersion as ConfigVersion;
            use crate::core::config::StarknetVersion as CoreVersion;
            config.madara.version = match version {
                CoreVersion::V0_13_2 => ConfigVersion::V0_13_2,
                CoreVersion::V0_13_3 => ConfigVersion::V0_13_3,
                CoreVersion::V0_13_4 => ConfigVersion::V0_13_4,
                CoreVersion::V0_13_5 => ConfigVersion::V0_13_5,
                CoreVersion::V0_14_0 => ConfigVersion::V0_14_0,
            };
        }
        if let Some(feeder_gateway) = &run_cmd.madara_feeder_gateway_url {
            debug!("Overriding madara.feeder_gateway_url from CLI");
            config.madara.feeder_gateway_url = Some(feeder_gateway.clone());
        }

        // Apply server overrides (these have defaults, so always present)
        debug!("Overriding server.host from CLI: {}", run_cmd.server_args.host);
        config.server.host = run_cmd.server_args.host.clone();

        debug!("Overriding server.port from CLI: {}", run_cmd.server_args.port);
        config.server.port = run_cmd.server_args.port;

        // Apply service overrides
        debug!("Overriding service.min_block_to_process from CLI: {}", run_cmd.service_args.min_block_to_process);
        config.service.min_block_to_process = run_cmd.service_args.min_block_to_process;

        if let Some(max_block) = run_cmd.service_args.max_block_to_process {
            debug!("Overriding service.max_block_to_process from CLI: {}", max_block);
            config.service.max_block_to_process = Some(max_block);
        }

        if let Some(max_concurrent_snos) = run_cmd.service_args.max_concurrent_snos_jobs {
            debug!("Overriding service.max_concurrent_snos_jobs from CLI: {}", max_concurrent_snos);
            config.service.max_concurrent_snos_jobs = Some(max_concurrent_snos);
        }

        if let Some(max_concurrent_proving) = run_cmd.service_args.max_concurrent_proving_jobs {
            debug!("Overriding service.max_concurrent_proving_jobs from CLI: {}", max_concurrent_proving);
            config.service.max_concurrent_proving_jobs = Some(max_concurrent_proving);
        }

        config.service.job_processing_timeout_seconds = run_cmd.service_args.job_processing_timeout_seconds;

        // Apply operational overrides
        if run_cmd.store_audit_artifacts {
            debug!("Overriding operational.store_audit_artifacts from CLI: true");
            config.operational.store_audit_artifacts = true;
        }
        config.operational.graceful_shutdown_timeout_seconds = run_cmd.graceful_shutdown_timeout;

        // Apply mock Atlantic server flag
        if run_cmd.mock_atlantic_server {
            debug!("Enabling mock Atlantic server from CLI");
            // This will be handled separately in the application startup
        }

        Ok(self)
    }

    /// Build and validate the final configuration
    pub fn build(self) -> Result<OrchestratorConfig> {
        let config = self.base_config.context("No configuration loaded")?;

        info!("Validating configuration");
        config.validate(&self.registry)?;

        info!("Configuration loaded and validated successfully");
        Ok(config)
    }
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper function to load configuration from RunCmd
/// This is the main entry point that determines the config source
///
/// BREAKING CHANGE: This function now requires either --config or --preset to be provided.
/// Legacy CLI-only mode is no longer supported.
pub fn load_config_from_run_cmd(run_cmd: &RunCmd) -> Result<OrchestratorConfig> {
    let builder = ConfigBuilder::new();

    // Require either --config or --preset
    let builder = if let Some(preset_name) = &run_cmd.preset {
        builder.with_preset(preset_name)?
    } else if let Some(config_path) = &run_cmd.config_file {
        builder.with_config_file(config_path)?
    } else {
        // No config source provided - this is now an error
        return Err(anyhow::anyhow!(
            "Configuration required: Please use --config or --preset to specify configuration.\n\n\
            Available presets:\n\
            • local-dev           - Local development environment\n\
            • l2-sepolia          - L2 deployment on Ethereum Sepolia\n\
            • l3-starknet-sepolia - L3 deployment on Starknet Sepolia\n\n\
            Examples:\n\
            • orchestrator run --preset local-dev\n\
            • orchestrator run --config /path/to/config.yaml\n\n\
            For more information, see the configuration documentation."
        ));
    };

    // Apply CLI overrides on top of config file/preset
    builder.with_cli_overrides(run_cmd)?.build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_builder_requires_base_config() {
        let builder = ConfigBuilder::new();
        let result = builder.build();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No configuration loaded"));
    }

    #[test]
    fn test_load_preset() {
        let builder = ConfigBuilder::new();
        let result = builder.with_preset("local-dev");
        assert!(result.is_ok());
    }
}
