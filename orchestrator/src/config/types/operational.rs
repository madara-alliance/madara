use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationalConfig {
    #[serde(default)]
    pub store_audit_artifacts: bool,

    #[serde(default = "default_shutdown_timeout")]
    pub graceful_shutdown_timeout_seconds: u64,

    #[serde(default)]
    pub mock_atlantic_server: bool,
}

fn default_shutdown_timeout() -> u64 {
    120
}
