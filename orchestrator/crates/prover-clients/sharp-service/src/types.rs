use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CairoJobStatus {
    #[default]
    Unknown,
    NotCreated,
    InProgress,
    Processed,
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

/// Body of `POST /v1/gateway/add_job`.
#[derive(Debug, Clone, Serialize)]
pub struct SharpAddJobRequest<'a> {
    /// Base64-encoded CairoPIE zip bytes.
    pub cairo_pie_encoded: &'a str,
}

/// Body of `POST /v1/gateway/add_applicative_job`.
#[derive(Debug, Clone, Serialize)]
pub struct SharpAddApplicativeJobRequest<'a> {
    /// Base64-encoded aggregator CairoPIE zip bytes.
    pub cairo_pie_encoded: &'a str,
    /// Child cairo_job_keys this applicative job aggregates, in applicative order.
    pub children_cairo_job_keys: &'a [String],
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
