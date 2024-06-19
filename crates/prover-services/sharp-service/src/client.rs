use serde_json::json;
use snos::sharp::{CairoJobResponse, CairoStatusResponse};
use url::Url;
use uuid::Uuid;

use crate::error::SharpError;

/// SHARP endpoint for Sepolia testnet
pub const DEFAULT_SHARP_URL: &str = "https://testnet.provingservice.io";

/// SHARP API async wrapper
pub struct SharpClient {
    base_url: Url,
    client: reqwest::Client,
}

impl SharpClient {
    pub fn new(url: Url) -> Self {
        Self { base_url: url, client: reqwest::Client::new() }
    }

    pub async fn add_job(&self, encoded_pie: &str) -> Result<CairoJobResponse, SharpError> {
        let data = json!({ "action": "add_job", "request": { "cairo_pie": encoded_pie } });
        let url = self.base_url.join("add_job").unwrap();
        let res = self.client.post(url).json(&data).send().await.map_err(SharpError::AddJobFailure)?;

        match res.status() {
            reqwest::StatusCode::OK => res.json().await.map_err(SharpError::AddJobFailure),
            code => Err(SharpError::SharpService(code)),
        }
    }

    pub async fn get_job_status(&self, job_key: &Uuid) -> Result<CairoStatusResponse, SharpError> {
        let data = json!({ "action": "get_status", "request": { "cairo_job_key": job_key } });
        let url = self.base_url.join("get_status").unwrap();
        let res = self.client.post(url).json(&data).send().await.map_err(SharpError::GetJobStatusFailure)?;

        match res.status() {
            reqwest::StatusCode::OK => res.json().await.map_err(SharpError::GetJobStatusFailure),
            code => Err(SharpError::SharpService(code)),
        }
    }
}

impl Default for SharpClient {
    fn default() -> Self {
        Self::new(DEFAULT_SHARP_URL.parse().unwrap())
    }
}
