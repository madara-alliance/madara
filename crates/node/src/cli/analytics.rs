use clap::Args;
use url::Url;

/// Parameters used to config analytics.
#[derive(Debug, Clone, Args)]
pub struct AnalyticsParams {
    /// Name of the service.
    #[arg(env = "MADARA_ANALYTICS_SERVICE_NAME", long, alias = "analytics", default_value = "board")]
    pub analytics_service_name: String,

    /// Log level of the service.
    #[arg(env = "RUST_LOG", long, default_value = "info")]
    pub analytics_log_level: String,

    /// Endpoint of the analytics server.
    #[arg(env = "OTEL_EXPORTER_OTLP_ENDPOINT", long, value_parser = parse_collection_endpoint, default_value = "http://localhost:4317")]
    pub analytics_collection_endpoint: Url,
}

#[derive(Debug, thiserror::Error)]
enum AnalyticsError {
    #[error("invalid collection endpoint: {0}")]
    CollectionEndpointParsingError(String),
}

fn parse_collection_endpoint(s: &str) -> Result<Url, AnalyticsError> {
    Url::parse(s).map_err(|_| AnalyticsError::CollectionEndpointParsingError(s.to_string()))
}
