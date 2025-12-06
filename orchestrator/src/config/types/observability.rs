use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservabilityConfig {
    pub otel: OTELConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OTELConfig {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default = "default_service_name")]
    pub service_name: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub collector_endpoint: Option<Url>,
}

fn default_service_name() -> String {
    "orchestrator".to_string()
}
