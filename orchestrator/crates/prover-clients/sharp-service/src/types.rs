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

impl CairoJobStatus {
    /// Bounded, stable label for Prometheus metrics.
    ///
    /// Decoupled from `Debug` so a variant rename (routine refactor) can't
    /// silently change the metric label surface and break Grafana queries
    /// / alerts built on these strings.
    pub fn as_label(&self) -> &'static str {
        match self {
            CairoJobStatus::Unknown => "unknown",
            CairoJobStatus::NotCreated => "not_created",
            CairoJobStatus::InProgress => "in_progress",
            CairoJobStatus::Processed => "processed",
            CairoJobStatus::Invalid => "invalid",
            CairoJobStatus::Failed => "failed",
        }
    }
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
