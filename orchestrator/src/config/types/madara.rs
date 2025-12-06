use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MadaraConfig {
    /// Madara RPC URL
    pub rpc_url: Url,

    /// Madara feeder gateway URL (defaults to rpc_url if not set)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feeder_gateway_url: Option<Url>,

    /// Starknet version
    pub version: StarknetVersion,
}

impl MadaraConfig {
    /// Get the feeder gateway URL (returns rpc_url if not explicitly set)
    pub fn feeder_gateway_url(&self) -> &Url {
        self.feeder_gateway_url.as_ref().unwrap_or(&self.rpc_url)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum StarknetVersion {
    #[serde(rename = "0.13.2")]
    V0_13_2,
    #[serde(rename = "0.13.3")]
    V0_13_3,
    #[serde(rename = "0.13.4")]
    V0_13_4,
    #[serde(rename = "0.13.5")]
    V0_13_5,
    #[serde(rename = "0.14.0")]
    V0_14_0,
}

impl StarknetVersion {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::V0_13_2 => "0.13.2",
            Self::V0_13_3 => "0.13.3",
            Self::V0_13_4 => "0.13.4",
            Self::V0_13_5 => "0.13.5",
            Self::V0_14_0 => "0.14.0",
        }
    }

    pub fn supported() -> &'static [StarknetVersion] {
        &[Self::V0_13_2, Self::V0_13_3, Self::V0_13_4, Self::V0_13_5, Self::V0_14_0]
    }

    pub fn is_supported(&self) -> bool {
        Self::supported().contains(self)
    }
}

impl Default for StarknetVersion {
    fn default() -> Self {
        Self::V0_14_0
    }
}

impl std::fmt::Display for StarknetVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_serialization() {
        let yaml = r#""0.14.0""#;
        let version: StarknetVersion = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(version, StarknetVersion::V0_14_0);
    }

    #[test]
    fn test_feeder_gateway_fallback() {
        let config = MadaraConfig {
            rpc_url: "http://localhost:9944".parse().unwrap(),
            feeder_gateway_url: None,
            version: StarknetVersion::V0_14_0,
        };

        assert_eq!(config.feeder_gateway_url(), &config.rpc_url);
    }
}
