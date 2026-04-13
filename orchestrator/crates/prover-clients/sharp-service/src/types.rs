use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CairoJobStatus {
    #[default]
    Unknown,
    NotCreated,
    InProgress,
    Processed,
    Onchain,
    Invalid,
    Failed,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum InvalidReason {
    #[default]
    Unknown,
    InvalidCairoPieFileFormat,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct SharpAddJobResponse {
    pub code: Option<String>,
    pub message: Option<String>,
}

#[derive(Default, Debug, Clone, Deserialize)]
pub struct SharpGetStatusResponse {
    #[serde(default)]
    pub status: CairoJobStatus,
    pub invalid_reason: Option<InvalidReason>,
    pub error_log: Option<String>,
    pub validation_done: Option<bool>,
}

#[derive(Default, Debug, Clone, Deserialize)]
pub struct SharpGetProofResponse {
    pub code: Option<String>,
}
