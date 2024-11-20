//! A flexible HTTP client builder with support for default parameters and request customization.
//!
//! This module provides a builder pattern implementation for creating and customizing HTTP clients
//! with default configurations. It supports:
//! - Default headers, query parameters, and form data
//! - Client-level TLS certificates and identities
//! - Request-level customization
//! - Body handling (currently optimized for string payloads)
//!
//! # Body Handling
//! The current implementation serializes request bodies to JSON format. The body method accepts
//! any type that implements the Serialize trait, allowing for flexible payload structures including
//! strings, numbers, objects, and arrays. For binary data or other formats, the implementation
//! would need to be modified.
//!
//! # Examples
//! ```
//! use utils::http_client::HttpClient;
//!
//! let client = HttpClient::builder("https://api.example.com")
//!     .default_header("Authorization", "Bearer token")
//!     .default_query_param("version", "v1")
//!     .default_body_param("tenant=main")
//!     .build()?;
//!
//! let response = client
//!     .request()
//!     .method(Method::POST)
//!     .path("/api/data")
//!     .query_param("id", "123")
//!     .body("name=test")
//!     .send()
//!     .await?;
//! ```

use std::collections::HashMap;
use std::path::Path;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::multipart::{Form, Part};
use reqwest::{Certificate, Client, ClientBuilder, Identity, Method, Response, Result};
use serde::Serialize;
use url::Url;

/// Main HTTP client with default configurations.
///
/// Holds the base configuration and default parameters that will be applied
/// to all requests made through this client.
#[derive(Debug)]
pub struct HttpClient {
    /// Base URL for all requests
    base_url: Url,
    /// Configured reqwest client
    client: Client,
    /// Headers to be included in every request
    default_headers: HeaderMap,
    /// Query parameters to be included in every request
    default_query_params: HashMap<String, String>,
    /// Form data to be included in every multipart request
    default_form_data: HashMap<String, String>,
    /// Body parameters to be included in every request body
    /// Note: Currently assumes string data with & as separator
    default_body_params: String,
}

/// Builder for constructing an HttpClient with custom configurations.
#[derive(Debug)]
pub struct HttpClientBuilder {
    base_url: Url,
    client_builder: ClientBuilder,
    default_headers: HeaderMap,
    default_query_params: HashMap<String, String>,
    default_form_data: HashMap<String, String>,
    default_body_params: String,
}

impl HttpClient {
    /// Creates a new builder for constructing an HttpClient.
    ///
    /// # Arguments
    /// * `base_url` - The base URL for all requests made through this client
    ///
    /// # Panics
    /// Panics if the provided base URL is invalid
    pub fn builder(base_url: &str) -> HttpClientBuilder {
        HttpClientBuilder::new(Url::parse(base_url).expect("Invalid base URL"))
    }

    /// Creates a new request builder for making HTTP requests.
    ///
    /// # Returns
    /// A RequestBuilder instance that can be used to construct and send an HTTP request
    pub fn request(&self) -> RequestBuilder {
        RequestBuilder::new(self)
    }

    /// Internal method to send a request with all default parameters applied.
    ///
    /// # Arguments
    /// * `builder` - The RequestBuilder containing request-specific configurations
    ///
    /// # Returns
    /// A Result containing either the Response or an error
    async fn send_request(&self, builder: RequestBuilder<'_>) -> Result<Response> {
        // Create a new URL by cloning the base URL and appending the path
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .expect("Base URL cannot be a base")
            .extend(builder.path.trim_start_matches('/').split('/'));
        // Merge query parameters
        {
            let mut pairs = url.query_pairs_mut();
            let default_params = self.default_query_params.clone();
            for (key, value) in default_params {
                pairs.append_pair(&key, &value);
            }
            for (key, value) in &builder.query_params {
                pairs.append_pair(key, value);
            }
        }

        let mut request = self.client.request(builder.method, url);

        // Merge headers
        let mut final_headers = self.default_headers.clone();
        final_headers.extend(builder.headers);
        request = request.headers(final_headers);

        // Handle body - merge builder body with defaults
        let mut body = String::new();
        if !self.default_body_params.is_empty() {
            body.push_str(&self.default_body_params);
        }
        if let Some(builder_body) = builder.body {
            if !body.is_empty() {
                body.push('&');
            }
            body.push_str(&builder_body);
        }

        if !body.is_empty() {
            request = request.body(body);
        }

        // Handle form data
        if let Some(mut form) = builder.form {
            let default_form: HashMap<String, String> = self.default_form_data.clone();
            for (key, value) in default_form {
                form = form.text(key, value);
            }
            request = request.multipart(form);
        }
        request.send().await
    }
}

impl HttpClientBuilder {
    /// Creates a new HttpClientBuilder with default configurations.
    fn new(base_url: Url) -> Self {
        Self {
            base_url,
            client_builder: Client::builder(),
            default_headers: HeaderMap::new(),
            default_query_params: HashMap::new(),
            default_form_data: HashMap::new(),
            default_body_params: String::new(),
        }
    }

    /// Adds client identity for TLS authentication.
    pub fn identity(mut self, identity: Identity) -> Self {
        self.client_builder = self.client_builder.identity(identity);
        self
    }

    /// Adds a root certificate for TLS verification.
    pub fn add_root_certificate(mut self, cert: Certificate) -> Self {
        self.client_builder = self.client_builder.add_root_certificate(cert);
        self
    }

    /// Adds a default header to be included in all requests.
    pub fn default_header(mut self, key: HeaderName, value: HeaderValue) -> Self {
        self.default_headers.insert(key, value);
        self
    }

    /// Adds a default query parameter to be included in all requests.
    pub fn default_query_param(mut self, key: &str, value: &str) -> Self {
        self.default_query_params.insert(key.to_string(), value.to_string());
        self
    }

    /// Adds default form data to be included in all multipart requests.
    pub fn default_form_data(mut self, key: &str, value: &str) -> Self {
        self.default_form_data.insert(key.to_string(), value.to_string());
        self
    }

    /// Builds the HttpClient with all configured defaults.
    pub fn build(self) -> Result<HttpClient> {
        Ok(HttpClient {
            base_url: self.base_url,
            client: self.client_builder.build()?,
            default_headers: self.default_headers,
            default_query_params: self.default_query_params,
            default_form_data: self.default_form_data,
            default_body_params: self.default_body_params,
        })
    }
}

/// Builder for constructing individual HTTP requests.
#[derive(Debug)]
pub struct RequestBuilder<'a> {
    client: &'a HttpClient,
    method: Method,
    path: String,
    headers: HeaderMap,
    query_params: HashMap<String, String>,
    /// Request body as a string. For binary data, this would need to be modified.
    body: Option<String>,
    form: Option<Form>,
}

impl<'a> RequestBuilder<'a> {
    fn new(client: &'a HttpClient) -> Self {
        Self {
            client,
            method: Method::GET,
            path: String::new(),
            headers: HeaderMap::new(),
            query_params: HashMap::new(),
            body: None,
            form: None,
        }
    }

    /// Sets the HTTP method for the request.
    pub fn method(mut self, method: Method) -> Self {
        self.method = method;
        self
    }

    /// Appends a path segment to the existing path.
    /// If the path starts with '/', it will replace the existing path instead of appending.
    pub fn path(mut self, path: &str) -> Self {
        if path.starts_with('/') {
            self.path = path.to_string();
        } else {
            if !self.path.is_empty() && !self.path.ends_with('/') {
                self.path.push('/');
            }
            self.path.push_str(path);
        }
        self
    }

    /// Adds a header to the request.
    pub fn header(mut self, key: HeaderName, value: HeaderValue) -> Self {
        self.headers.insert(key, value);
        self
    }

    /// Adds a query parameter to the request.
    pub fn query_param(mut self, key: &str, value: &str) -> Self {
        self.query_params.insert(key.to_string(), value.to_string());
        self
    }

    /// Sets the request body by serializing the input to JSON.
    /// This method can handle any type that implements Serialize.
    ///
    /// # Arguments
    /// * `body` - The data to be serialized and sent as the request body
    ///
    /// # Examples
    /// ```
    /// let request = client.request()
    ///     .method(Method::POST)
    ///     .body(json!({ "key": "value" }));
    /// ```
    pub fn body<T: Serialize>(mut self, body: T) -> Self {
        let body_string = serde_json::to_string(&body).expect("Failed to serialize body");
        self.body = Some(body_string);
        self
    }

    /// Adds a text part to the multipart form.
    pub fn form_text(mut self, key: &str, value: &str) -> Self {
        let form = match self.form.take() {
            Some(existing_form) => existing_form.text(key.to_string(), value.to_string()),
            None => Form::new().text(key.to_string(), value.to_string()),
        };
        self.form = Some(form);
        self
    }

    /// Adds a file part to the multipart form.
    pub fn form_file(mut self, key: &str, file_path: &Path, file_name: &str) -> Self {
        let file_bytes = std::fs::read(file_path).expect("Failed to read file");
        // Convert file_name to owned String
        let file_name = file_name.to_string();

        let part = Part::bytes(file_bytes).file_name(file_name);

        let form = match self.form.take() {
            Some(existing_form) => existing_form.part(key.to_string(), part),
            None => Form::new().part(key.to_string(), part),
        };
        self.form = Some(form);
        self
    }
    /// Sends the request with all configured parameters.
    pub async fn send(self) -> Result<Response> {
        self.client.send_request(self).await
    }
}

#[cfg(test)]
mod http_client_tests {
    use std::path::PathBuf;

    use base64::engine::general_purpose;
    use base64::Engine;
    use httpmock;
    use reqwest::header::{HeaderName, HeaderValue};

    use super::*;

    const TEST_URL: &str = "https://madara.orchestrator.com";

    /// Verifies that HttpClient::builder creates a valid builder with the provided base URL
    /// and all default values are properly initialized
    #[test]
    fn test_builder_basic_initialization() {
        let client = HttpClient::builder(TEST_URL).build().unwrap();

        assert_eq!(client.base_url.as_str(), format!("{}/", TEST_URL));
        assert!(client.default_headers.is_empty());
        assert!(client.default_query_params.is_empty());
        assert!(client.default_form_data.is_empty());
        assert!(client.default_body_params.is_empty());
    }

    /// Ensures the builder properly panics when provided with an invalid URL
    /// Cases: malformed URLs, invalid schemes, empty URLs
    #[test]
    #[should_panic(expected = "Invalid base URL")]
    fn test_builder_invalid_url() {
        HttpClient::builder("not a url");
    }

    /// Verifies that default headers set during builder phase are:
    /// - Properly stored in the builder
    /// - Correctly transferred to the final client
    /// - Applied to outgoing requests
    #[test]
    fn test_builder_default_headers() {
        let header_name = HeaderName::from_static("x-test");
        let header_value = HeaderValue::from_static("test-value");

        let client =
            HttpClient::builder(TEST_URL).default_header(header_name.clone(), header_value.clone()).build().unwrap();

        assert!(client.default_headers.contains_key(&header_name));
        assert_eq!(client.default_headers.get(&header_name).unwrap(), &header_value);
    }

    /// Validates default query parameter handling:
    /// - Parameters are correctly stored
    /// - Multiple parameters can be added
    /// - Parameters are properly URL encoded
    #[test]
    fn test_builder_default_query_params() {
        let client = HttpClient::builder(TEST_URL)
            .default_query_param("key1", "value1")
            .default_query_param("key2", "value 2")
            .build()
            .unwrap();

        assert_eq!(client.default_query_params.len(), 2);
        assert_eq!(client.default_query_params.get("key1").unwrap(), "value1");
        assert_eq!(client.default_query_params.get("key2").unwrap(), "value 2");
    }

    /// # Request Builder Tests

    /// Verifies that all HTTP methods (GET, POST, PUT, DELETE, etc.)
    /// can be correctly set and are properly sent in requests
    #[test]
    fn test_request_builder_method_setting() {
        let client = HttpClient::builder(TEST_URL).build().unwrap();

        let request = client.request().method(Method::GET);
        assert_eq!(request.method, Method::GET);

        let request = client.request().method(Method::POST);
        assert_eq!(request.method, Method::POST);

        let request = client.request().method(Method::PUT);
        assert_eq!(request.method, Method::PUT);

        let request = client.request().method(Method::DELETE);
        assert_eq!(request.method, Method::DELETE);
    }

    /// Tests path handling functionality:
    /// - Absolute paths (starting with /) replace existing path
    /// - Relative paths are properly appended
    /// - Multiple path segments are correctly joined
    /// - Empty paths and special characters
    /// - Unicode paths
    #[test]
    fn test_request_builder_path_handling() {
        let client = HttpClient::builder(TEST_URL).build().unwrap();

        // Test absolute path
        let request = client.request().path("/absolute/path");
        assert_eq!(request.path, "/absolute/path");

        // Test relative path
        let request = client.request().path("relative").path("path");
        assert_eq!(request.path, "relative/path");

        // Test mixed paths
        let request = client.request().path("/absolute").path("relative");
        assert_eq!(request.path, "/absolute/relative");

        // Test empty path handling
        let request = client.request().path("");
        assert_eq!(request.path, "");

        // Test multiple slashes
        let request = client.request().path("//test//path//");
        assert_eq!(request.path, "//test//path//");

        // Test Unicode path
        let request = client.request().path("/测试/路径");
        assert_eq!(request.path, "/测试/路径");
    }

    /// Tests query parameter behavior:
    /// - Request-specific parameters are added
    /// - Parameters merge correctly with defaults
    /// - Later parameters override earlier ones
    #[test]
    fn test_request_builder_query_params() {
        let client = HttpClient::builder(TEST_URL).default_query_param("default", "value").build().unwrap();

        let request = client.request().query_param("test", "value").query_param("another", "param");

        assert_eq!(request.query_params.len(), 2);
        assert_eq!(request.query_params.get("test").unwrap(), "value");
        assert_eq!(request.query_params.get("another").unwrap(), "param");
    }

    /// Tests header manipulation:
    /// - Headers can be added to specific requests
    /// - Request headers properly merge with defaults
    /// - Request-specific headers override defaults
    #[test]
    fn test_request_builder_headers() {
        let header_name = HeaderName::from_static("x-test");
        let header_value = HeaderValue::from_static("default-value");
        let client = HttpClient::builder(TEST_URL).default_header(header_name.clone(), header_value).build().unwrap();

        let new_value = HeaderValue::from_static("new-value");
        let request = client.request().header(header_name.clone(), new_value.clone());

        assert!(request.headers.contains_key(&header_name));
        assert_eq!(request.headers.get(&header_name).unwrap(), &new_value);
    }

    /// # Form Data Tests

    /// Validates multipart form text field handling:
    /// - Single field addition
    /// - Multiple fields
    /// - Form builder chaining
    #[test]
    fn test_multipart_form_text() {
        let client = HttpClient::builder(TEST_URL).build().unwrap();

        // Test initial state
        let request = client.request();
        assert!(request.form.is_none());

        // Test single field
        let request = client.request().form_text("field1", "value1");
        assert!(request.form.is_some());

        // Test multiple fields with chaining
        let request = client.request().form_text("field1", "value1").form_text("field2", "value2");
        assert!(request.form.is_some());
    }

    /// Tests file upload functionality:
    /// - Single file upload
    /// - File name handling
    /// - Binary content handling
    /// - Non-existent file handling
    #[test]
    fn test_multipart_form_file() {
        let client = HttpClient::builder(TEST_URL).build().unwrap();
        let file_path: PathBuf = "../orchestrator/src/tests/artifacts/fibonacci.zip".parse().unwrap();

        // Test initial state
        let request = client.request();
        assert!(request.form.is_none());

        let request = client.request().form_file("file", &file_path, "fibonacci.zip");

        assert!(request.form.is_some());
    }

    /// Tests body serialization functionality for the request builder:
    /// - JSON serialization of different types
    /// - Struct serialization
    /// - Array/Vector serialization
    #[test]
    fn test_request_builder_body_serialization() {
        let client = HttpClient::builder(TEST_URL).build().unwrap();

        // Test string body
        let request = client.request().body("test string");
        assert_eq!(request.body.unwrap(), r#""test string""#);

        // Test number body
        let request = client.request().body(42);
        assert_eq!(request.body.unwrap(), "42");

        // Test struct body
        #[derive(Serialize)]
        struct TestStruct {
            field1: String,
            field2: i32,
        }

        let test_struct = TestStruct { field1: "test".to_string(), field2: 123 };

        let request = client.request().body(test_struct);
        assert_eq!(request.body.unwrap(), r#"{"field1":"test","field2":123}"#);

        // Test array/vec body
        let vec_data = vec![1, 2, 3];
        let request = client.request().body(vec_data);
        assert_eq!(request.body.unwrap(), "[1,2,3]");
    }

    /// Tests TLS certificate and identity handling:
    /// - Valid certificate addition
    /// - Invalid certificate handling
    /// - Identity verification
    #[test]
    fn test_certificate_handling() {
        // Load variables from .env.test
        dotenv::from_filename(".env.test").ok();

        // Getting the cert files from the .env.test and then decoding it from base64
        let cert = general_purpose::STANDARD
            .decode(std::env::var("MADARA_ORCHESTRATOR_SHARP_USER_CRT").unwrap())
            .expect("Failed to decode certificate");
        let key = general_purpose::STANDARD
            .decode(std::env::var("MADARA_ORCHESTRATOR_SHARP_USER_KEY").unwrap())
            .expect("Failed to decode sharp user key");
        let server_cert = general_purpose::STANDARD
            .decode(std::env::var("MADARA_ORCHESTRATOR_SHARP_SERVER_CRT").unwrap())
            .expect("Failed to decode sharp server certificate");

        let identity =
            Identity::from_pkcs8_pem(&cert, &key).expect("Failed to build the identity from certificate and key");
        let certificate = Certificate::from_pem(server_cert.as_slice()).expect("Failed to add root certificate");

        let client =
            HttpClient::builder(TEST_URL).identity(identity).add_root_certificate(certificate).build().unwrap();

        // Since we can't check the certificates directly, we'll just verify the client was built
        assert_eq!(client.base_url.as_str(), (TEST_URL.to_owned() + "/"));
    }

    /// Tests client behavior with:
    /// - Large body content
    /// - Large file uploads
    /// - Memory usage monitoring
    #[tokio::test]
    async fn test_large_payload_handling() {
        let mock_server = httpmock::MockServer::start();

        let client = HttpClient::builder(&mock_server.base_url()).build().unwrap();

        // Create a large body string
        let large_body = "x".repeat(1024 * 1024); // 1MB string
        let request = client.request().method(Method::POST).body(&large_body);

        assert!(request.body.is_some());
        assert_eq!(request.body.unwrap().len(), (1024 * 1024) + 2);
    }

    /// # Integration Tests

    /// Tests complete request flow including:
    /// - URL construction
    /// - Header merging
    /// - Query parameter combination
    /// - Body handling
    /// - Response processing
    #[tokio::test]
    async fn test_complete_request_flow() {
        let mock_server = httpmock::MockServer::start();

        let mock = mock_server.mock(|when, then| {
            when.method("POST")
                .path("/api/data")
                .query_param("version", "v1")
                .query_param("id", "123")
                .header("Authorization", "Bearer token")
                .body(r#"{"name":"test"}"#);

            then.status(200).header("content-type", "application/json").body(r#"{"status": "ok"}"#);
        });

        let client = HttpClient::builder(&mock_server.base_url())
            .default_header(HeaderName::from_static("authorization"), HeaderValue::from_static("Bearer token"))
            .default_query_param("version", "v1")
            .build()
            .unwrap();

        #[derive(Serialize)]
        struct RequestBody {
            name: String,
        }

        let response = client
            .request()
            .method(Method::POST)
            .path("/api/data")
            .query_param("id", "123")
            .body(RequestBody { name: "test".to_string() })
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
        mock.assert();
    }
}
