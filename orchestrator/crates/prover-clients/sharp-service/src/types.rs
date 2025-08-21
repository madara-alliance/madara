use serde::{Deserialize, Serialize};
use starknet_os::sharp::{CairoJobStatus, InvalidReason};

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct SharpAddJobResponse {
    pub code: Option<String>,
    pub message: Option<String>,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct SharpGetProofResponse {
    pub code: Option<String>,
}

#[derive(Default, Debug, Clone, Deserialize)]
pub struct SharpGetStatusResponse {
    #[serde(default)]
    pub status: CairoJobStatus,
    pub invalid_reason: Option<InvalidReason>,
    pub error_log: Option<String>,
    pub validation_done: Option<bool>,
}

/// **IMPORTANT NOTE: THIS IS A MOCK RESPONSE FOR E2E TEST**
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct SharpCreateBucketResponse {
    pub code: String,
    pub bucket_id: String,
}

/// **IMPORTANT NOTE: THIS IS A MOCK RESPONSE FOR E2E TEST**
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct SharpGetAggTaskIdResponse {
    pub task_id: String,
}
