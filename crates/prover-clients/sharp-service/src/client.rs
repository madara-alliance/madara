use base64::engine::general_purpose;
use base64::Engine;
use cairo_vm::types::layout_name::LayoutName;
use reqwest::{Certificate, Identity, Method, StatusCode};
use url::Url;
use utils::http_client::HttpClient;
use uuid::Uuid;

use crate::error::SharpError;
use crate::types::{SharpAddJobResponse, SharpGetStatusResponse};
use crate::SharpValidatedArgs;

/// SHARP API async wrapper
pub struct SharpClient {
    client: HttpClient,
}

impl SharpClient {
    /// We need to set up the client with the provided certificates.
    /// We need to have three secrets :
    /// - base64(SHARP_USER_CRT)
    /// - base64(SHARP_USER_KEY)
    /// - base64(SHARP_SERVER_CRT)
    ///
    /// You can run this command in terminal to convert a file output into base64
    /// and then copy it and paste it into .env file :
    ///
    /// `cat <file_name> | base64`
    pub fn new_with_args(url: Url, sharp_params: &SharpValidatedArgs) -> Self {
        // Getting the cert files from the .env and then decoding it from base64

        let cert = general_purpose::STANDARD
            .decode(sharp_params.sharp_user_crt.clone())
            .expect("Failed to decode certificate");
        let key = general_purpose::STANDARD
            .decode(sharp_params.sharp_user_key.clone())
            .expect("Failed to decode sharp user key");
        let server_cert = general_purpose::STANDARD
            .decode(sharp_params.sharp_server_crt.clone())
            .expect("Failed to decode sharp server certificate");

        let customer_id = sharp_params.sharp_customer_id.clone();

        let identity =
            Identity::from_pkcs8_pem(&cert, &key).expect("Failed to build the identity from certificate and key");
        let certificate = Certificate::from_pem(server_cert.as_slice()).expect("Failed to add root certificate");

        let client = HttpClient::builder(url.as_str())
            .identity(identity)
            .add_root_certificate(certificate)
            .default_query_param("customer_id", customer_id.as_str())
            .build()
            .expect("Failed to build the http client");

        Self { client }
    }

    pub async fn add_job(
        &self,
        encoded_pie: &str,
        proof_layout: LayoutName,
    ) -> Result<(SharpAddJobResponse, Uuid), SharpError> {
        let cairo_key = Uuid::new_v4();

        let proof_layout = match proof_layout {
            LayoutName::dynamic => "dynamic",
            _ => proof_layout.to_str(),
        };

        let response = self
            .client
            .request()
            .method(Method::POST)
            .path("add_job")
            .query_param("cairo_job_key", &cairo_key.to_string())
            .query_param("offchain_proof", "true")
            .query_param("proof_layout", proof_layout)
            .body(encoded_pie)
            .send()
            .await
            .map_err(SharpError::AddJobFailure)?;

        match response.status() {
            StatusCode::OK => {
                let result = response.json().await.map_err(SharpError::AddJobFailure)?;
                Ok((result, cairo_key))
            }
            code => Err(SharpError::SharpService(code)),
        }
    }

    pub async fn get_job_status(&self, job_key: &Uuid) -> Result<SharpGetStatusResponse, SharpError> {
        let response = self
            .client
            .request()
            .method(Method::POST)
            .path("get_status")
            .query_param("cairo_job_key", &job_key.to_string())
            .send()
            .await
            .map_err(SharpError::GetJobStatusFailure)?;

        match response.status() {
            StatusCode::OK => response.json().await.map_err(SharpError::GetJobStatusFailure),
            code => Err(SharpError::SharpService(code)),
        }
    }
}
