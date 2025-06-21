use clap::Args;
use mp_utils::parsers::parse_url;
use serde::{Deserialize, Serialize};
use url::Url;

/// Parameters used to config analytics.
#[derive(Debug, Clone, Args, Deserialize, Serialize)]
pub struct AnalyticsParams {
    /// Name of the service.
    #[arg(env = "MADARA_ANALYTICS_SERVICE_NAME", long, alias = "analytics", default_value = "madara_analytics")]
    pub analytics_service_name: String,

    /// Endpoint of the analytics server.
    #[arg(env = "OTEL_EXPORTER_OTLP_ENDPOINT", long, value_parser = parse_url, default_value = None)]
    pub analytics_collection_endpoint: Option<Url>,
}
