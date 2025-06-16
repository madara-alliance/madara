use crate::error::AtlanticError;
use crate::types::{
    AtlanticAddJobResponse, AtlanticCairoVersion, AtlanticCairoVm, AtlanticGetStatusResponse, AtlanticQueryStep,
};
use crate::AtlanticValidatedArgs;
use cairo_vm::types::layout_name::LayoutName;
use orchestrator_utils::http_client::{HttpClient, RequestBuilder};
use reqwest::Method;
use std::path::Path;
use tracing::debug;
use url::Url;

const ATLANTIC_PROOF_URL: &str = "https://s3.pl-waw.scw.cloud/atlantic-k8s-experimental/queries/{}/proof.json";

#[derive(Debug, strum_macros::EnumString)]
enum ProverType {
    #[strum(serialize = "starkware")]
    Starkware,
    #[strum(serialize = "herodotus")]
    HeroDotus,
}

trait ProvingLayer: Send + Sync {
    fn customize_request<'a>(&self, request: RequestBuilder<'a>) -> RequestBuilder<'a>;
}

struct EthereumLayer;
impl ProvingLayer for EthereumLayer {
    fn customize_request<'a>(&self, request: RequestBuilder<'a>) -> RequestBuilder<'a> {
        request.form_text("result", &AtlanticQueryStep::ProofVerificationOnL1.to_string())
    }
}

struct StarknetLayer;
impl ProvingLayer for StarknetLayer {
    fn customize_request<'a>(&self, request: RequestBuilder<'a>) -> RequestBuilder<'a> {
        request.form_text("result", &AtlanticQueryStep::ProofGeneration.to_string())
    }
}

/// SHARP API async wrapper
pub struct AtlanticClient {
    client: HttpClient,
    proving_layer: Box<dyn ProvingLayer>,
}

impl AtlanticClient {
    /// We need to set up the client with the API_KEY.
    pub fn new_with_args(url: Url, atlantic_params: &AtlanticValidatedArgs) -> Self {
        let mock_fact_hash = atlantic_params.atlantic_mock_fact_hash.clone();
        let client = HttpClient::builder(url.as_str())
            .expect("Failed to create HTTP client builder")
            .default_form_data("mockFactHash", &mock_fact_hash)
            .build()
            .expect("Failed to build HTTP client");

        let proving_layer: Box<dyn ProvingLayer> = match atlantic_params.atlantic_settlement_layer.as_str() {
            "ethereum" => Box::new(EthereumLayer),
            "starknet" => Box::new(StarknetLayer),
            _ => panic!("Invalid settlement layer: {}", atlantic_params.atlantic_settlement_layer),
        };

        Self { client, proving_layer }
    }

    pub async fn add_job(
        &self,
        pie_file: &Path,
        proof_layout: LayoutName,
        atlantic_api_key: impl AsRef<str>,
        n_steps: Option<usize>,
        atlantic_network: impl AsRef<str>,
    ) -> Result<AtlanticAddJobResponse, AtlanticError> {
        let proof_layout = match proof_layout {
            LayoutName::dynamic => "dynamic",
            _ => proof_layout.to_str(),
        };

        debug!(
            "Submitting job with layout: {}, n_steps: {}, network: {}, ",
            proof_layout,
            self.n_steps_to_job_size(n_steps),
            atlantic_network.as_ref()
        );

        let api = self.proving_layer.customize_request(
            self.client
                .request()
                .method(Method::POST)
                .path("atlantic-query")
                .query_param("apiKey", atlantic_api_key.as_ref())
                .form_text("declaredJobSize", self.n_steps_to_job_size(n_steps))
                .form_text("layout", proof_layout)
                .form_text("cairoVersion", &AtlanticCairoVersion::Cairo0.as_str())
                .form_text("cairoVm", &AtlanticCairoVm::Rust.as_str())
                .form_file("pieFile", pie_file, "pie.zip", Some("application/zip"))?,
        );
        debug!("Triggering the debug Request for: {:?}", api);
        let response = api.send().await.map_err(AtlanticError::AddJobFailure)?;

        match response.status().is_success() {
            true => response.json().await.map_err(AtlanticError::AddJobFailure),
            false => Err(AtlanticError::AtlanticService(response.status())),
        }
    }

    pub async fn get_job_status(&self, job_key: &str) -> Result<AtlanticGetStatusResponse, AtlanticError> {
        let response = self
            .client
            .request()
            .method(Method::GET)
            .path("atlantic-query")
            .path(job_key)
            .send()
            .await
            .map_err(AtlanticError::GetJobStatusFailure)?;

        if response.status().is_success() {
            response.json().await.map_err(AtlanticError::GetJobStatusFailure)
        } else {
            Err(AtlanticError::AtlanticService(response.status()))
        }
    }

    // get_proof_by_task_id - is a endpoint to get the proof from the herodotus service
    // Args:
    // task_id - the task id of the proof to get
    // Returns:
    // The proof as a string if the request is successful, otherwise an error is returned
    pub async fn get_proof_by_task_id(&self, task_id: &str) -> Result<String, AtlanticError> {
        // Note: It seems this code will be replaced by the proper API once it is available
        debug!("Getting proof for task_id: {}", task_id);
        let proof_path = ATLANTIC_PROOF_URL.replace("{}", task_id);
        let client = reqwest::Client::new();
        let response = client.get(&proof_path).send().await.map_err(AtlanticError::GetJobResultFailure)?;

        if response.status().is_success() {
            let response_text = response.text().await.map_err(AtlanticError::GetJobResultFailure)?;
            Ok(response_text)
        } else {
            Err(AtlanticError::AtlanticService(response.status()))
        }
    }

    pub async fn submit_l2_query(
        &self,
        proof: &str,
        cairo_verifier: &str,
        n_steps: Option<usize>,
        atlantic_network: impl AsRef<str>,
        atlantic_api_key: &str,
    ) -> Result<AtlanticAddJobResponse, AtlanticError> {
        let proof_layout = LayoutName::recursive_with_poseidon.to_str();

        let response = self.client
            .request()
            .method(Method::POST)
            .path("atlantic-query")
            .query_param("apiKey", atlantic_api_key.as_ref())// payload is not needed for L2
            .form_file_bytes("inputFile", proof.as_bytes().to_vec(), "proof.json", Some("application/json"))?
            .form_file_bytes("programFile", cairo_verifier.as_bytes().to_vec(), "cairo_verifier.json", Some("application/json"))?
            .form_text("layout", proof_layout)
            .form_text("declaredJobSize", self.n_steps_to_job_size(n_steps))
            .form_text("network", atlantic_network.as_ref())
            .form_text("result", &AtlanticQueryStep::ProofVerificationOnL2.to_string())
            .form_text("cairoVm", &AtlanticCairoVm::Python.as_str())
            .form_text("cairoVersion", &AtlanticCairoVersion::Cairo0.as_str())
            .send()
            .await
            .map_err(AtlanticError::AddJobFailure)?;

        match response.status().is_success() {
            true => response.json().await.map_err(AtlanticError::AddJobFailure),
            false => Err(AtlanticError::AtlanticService(response.status())),
        }
    }

    // https://docs.herodotus.cloud/atlantic/sending-query#sending-query
    fn n_steps_to_job_size(&self, n_steps: Option<usize>) -> &'static str {
        let n_steps = n_steps.unwrap_or(40_000_000) / 1_000_000;

        match n_steps {
            0..=12 => "S",
            13..=29 => "M",
            _ => "L",
        }
    }
}
