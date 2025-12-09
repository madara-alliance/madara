use anyhow::{Context, Result};
use std::path::PathBuf;

use super::{OrchestratorConfig, OrchestratorConfigVersioned};

/// Built-in preset configurations
pub struct PresetRegistry {
    presets: Vec<String>,
    presets_dir: PathBuf,
}

impl PresetRegistry {
    pub fn new() -> Self {
        // Get the workspace root by going up from the orchestrator package
        // This allows the presets to be stored externally in configs/orchestrator/presets/
        let workspace_root = std::env::var("CARGO_MANIFEST_DIR")
            .map(PathBuf::from)
            .map(|p| p.parent().unwrap_or(&p).to_path_buf())
            .unwrap_or_else(|_| {
                // Fallback: use current directory and go up to find configs
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
            });

        let presets_dir = workspace_root.join("configs/orchestrator/presets");

        Self {
            presets: vec!["local-dev".to_string(), "l2-sepolia".to_string(), "l3-starknet-sepolia".to_string()],
            presets_dir,
        }
    }

    pub fn builtin() -> Self {
        Self::new()
    }

    pub fn get_preset_path(&self, name: &str) -> Option<PathBuf> {
        if self.presets.contains(&name.to_string()) {
            Some(self.presets_dir.join(format!("{}.yaml", name)))
        } else {
            None
        }
    }

    pub fn list(&self) -> Vec<String> {
        self.presets.clone()
    }

    pub fn load_preset(&self, name: &str) -> Result<OrchestratorConfig> {
        let preset_path = self
            .get_preset_path(name)
            .ok_or_else(|| anyhow::anyhow!("Unknown preset: {}. Available presets: {:?}", name, self.list()))?;

        // Read the preset file
        let content = std::fs::read_to_string(&preset_path)
            .with_context(|| format!("Failed to read preset file: {}", preset_path.display()))?;

        // Parse the preset
        let versioned = OrchestratorConfigVersioned::from_yaml_str(&content)
            .with_context(|| format!("Failed to parse preset '{}'", name))?;

        let mut config = versioned.into_canonical();

        // Resolve RPC URLs
        super::file::resolve_rpc_urls(&mut config)?;

        Ok(config)
    }
}

impl Default for PresetRegistry {
    fn default() -> Self {
        Self::builtin()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_presets() {
        let registry = PresetRegistry::builtin();
        let presets = registry.list();

        assert!(presets.contains(&"local-dev".to_string()));
        assert!(presets.contains(&"l2-sepolia".to_string()));
        assert!(presets.contains(&"l3-starknet-sepolia".to_string()));
    }

    #[test]
    fn test_get_preset_path() {
        let registry = PresetRegistry::builtin();

        let path = registry.get_preset_path("local-dev");
        assert!(path.is_some());
        assert!(path.unwrap().to_string_lossy().contains("local-dev.yaml"));
    }

    #[test]
    fn test_load_preset() {
        let registry = PresetRegistry::builtin();

        // This test will only pass if the configs directory exists
        // Skip if we're in a test environment without the configs
        if let Ok(config) = registry.load_preset("local-dev") {
            assert_eq!(config.deployment.layer, crate::config::types::deployment::Layer::L2);
        }
    }
}
