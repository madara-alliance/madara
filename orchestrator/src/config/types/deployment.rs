use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentConfig {
    /// Deployment name (e.g., "madara-sepolia-orchestrator")
    pub name: String,

    /// Layer type: l2 or l3
    pub layer: Layer,

    /// Environment: dev, staging, production
    #[serde(default = "default_environment")]
    pub environment: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layer {
    L2,
    L3,
}

impl std::fmt::Display for Layer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Layer::L2 => write!(f, "l2"),
            Layer::L3 => write!(f, "l3"),
        }
    }
}

fn default_environment() -> String {
    "dev".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layer_serialization() {
        let yaml = r#"l2"#;
        let layer: Layer = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(layer, Layer::L2);

        let yaml = r#"l3"#;
        let layer: Layer = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(layer, Layer::L3);
    }
}
