use anyhow::{Context, Result};
use std::collections::HashMap;

use super::{OrchestratorConfig, OrchestratorConfigVersioned};

/// Built-in preset configurations
pub struct PresetRegistry {
    presets: HashMap<String, &'static str>,
}

impl PresetRegistry {
    pub fn builtin() -> Self {
        let mut presets = HashMap::new();

        // Register built-in presets
        presets.insert("local-dev".to_string(), include_str!("presets/local-dev.yaml"));
        presets.insert("l2-sepolia".to_string(), include_str!("presets/l2-sepolia.yaml"));
        presets.insert("l3-starknet-sepolia".to_string(), include_str!("presets/l3-starknet-sepolia.yaml"));

        Self { presets }
    }

    pub fn get(&self, name: &str) -> Option<&'static str> {
        self.presets.get(name).copied()
    }

    pub fn list(&self) -> Vec<String> {
        let mut names: Vec<_> = self.presets.keys().cloned().collect();
        names.sort();
        names
    }

    pub fn load_preset(&self, name: &str) -> Result<OrchestratorConfig> {
        let content = self
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Unknown preset: {}. Available presets: {:?}", name, self.list()))?;

        // Parse the preset
        let versioned = OrchestratorConfigVersioned::from_yaml_str(content)
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
    fn test_get_preset() {
        let registry = PresetRegistry::builtin();

        let content = registry.get("local-dev");
        assert!(content.is_some());
        assert!(content.unwrap().contains("config_version"));
    }
}
