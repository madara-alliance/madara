use std::path::Path;

use cairo_vm::types::layout_name::LayoutName;
use orchestrator_utils::http_client::{HttpClient, RequestBuilder};
use reqwest::Method;
use url::Url;

use crate::error::AtlanticError;
use crate::types::{
    AtlanticAddJobResponse, AtlanticCairoVersion, AtlanticCairoVm, AtlanticGetStatusResponse, AtlanticQueryStep,
};
use crate::AtlanticValidatedArgs;

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
        request.form_text("result", &AtlanticQueryStep::ProofVerificationOnL2.to_string())
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

        let response = self
            .proving_layer
            .customize_request(
                self.client
                    .request()
                    .method(Method::POST)
                    .path("atlantic-query")
                    .query_param("apiKey", atlantic_api_key.as_ref())
                    .form_text("declaredJobSize", self.n_steps_to_job_size(n_steps))
                    .form_text("layout", proof_layout)        
                    .form_text("result", "PROOF_GENERATION")
                    .form_text("network", atlantic_network.as_ref())
                    .form_text("cairoVersion", &AtlanticCairoVersion::Cairo0.as_str())
                    .form_text("cairoVm", &AtlanticCairoVm::Rust.as_str())
                    .form_file("pieFile", pie_file, "pie.zip")?,
            )
            .send()
            .await
            .map_err(AtlanticError::AddJobFailure)?;

        match response.status().is_success() {
            true => response.json().await.map_err(AtlanticError::AddJobFailure),
            false => Err(AtlanticError::SharpService(response.status())),
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
            Err(AtlanticError::SharpService(response.status()))
        }
    }

    pub async fn submit_l2_query(
        &self,
        proof: &str,
        n_steps: Option<usize>,
        atlantic_network: impl AsRef<str>,
        atlantic_api_key: &str,
    ) -> Result<AtlanticAddJobResponse, AtlanticError> {

        let cairo_verifier = match tokio::fs::read_to_string("build/cairo_verifier.json").await {
            Ok(content) => content,
            Err(e) => return Err(AtlanticError::FileReadError(e)),
        };
        
        let response = self
            .proving_layer
            .customize_request(
                self.client.request().method(Method::POST).query_param("apiKey", atlantic_api_key.as_ref()),
            )
            // doesn't seem like a valid input
            // .form_text("programHash", "0x193641eb151b0f41674641089952e60bc3aded26e3cf42793655c562b8c3aa0")
            // .form_text("prover", "starkware_sharp")
            .form_file_bytes("inputFile", proof.as_bytes().to_vec(), "proof.json")
            .form_file_bytes("programFile", cairo_verifier.as_bytes().to_vec(), "cairo_verifier.json")
            .form_text("layout", "recursive_with_poseidon")
            .form_text("declaredJobSize", self.n_steps_to_job_size(n_steps))
            .form_text("network", atlantic_network.as_ref())
            .form_text("result", "PROOF_VERIFICATION_ON_L2")
            .form_text("cairoVm", &AtlanticCairoVm::Rust.as_str())
            .form_text("cairoVersion", &AtlanticCairoVersion::Cairo0.as_str())
            .send()
            .await
            .map_err(AtlanticError::SubmitL2QueryFailure)?;

        match response.status().is_success() {
            true => response.json().await.map_err(AtlanticError::AddJobFailure),
            false => Err(AtlanticError::SharpService(response.status())),
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
