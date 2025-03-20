use clap::Args;
use mc_analytics::{AnalyticsConfig, PrometheusEndpointConfig};
use mp_utils::parsers::parse_url;
use url::Url;

/// Parameters used to config analytics.
#[derive(Debug, Clone, Args)]
pub struct AnalyticsParams {
    /// Name of the service.
    #[arg(env = "MADARA_ANALYTICS_SERVICE_NAME", long, alias = "analytics", default_value = "madara_analytics")]
    pub analytics_service_name: String,

    /// Endpoint of the analytics server.
    #[arg(env = "OTEL_EXPORTER_OTLP_ENDPOINT", long, value_parser = parse_url, default_value = None)]
    pub analytics_collection_endpoint: Option<Url>,

    /// Enable the prometheus endpoint. Exporting metrics using OTEL will not work if this is enabled.
    #[arg(env = "MADARA_ANALYTICS_PROMETHEUS_ENDPOINT", long)]
    pub analytics_prometheus_endpoint: bool,

    /// Listen on all network interfaces. This usually means the server will be accessible externally.
    #[arg(env = "MADARA_ANALYTICS_PROMETHEUS_ENDPOINT_EXTERNAL", long)]
    pub analytics_prometheus_endpoint_external: bool,

    /// The port to listen on.
    #[arg(env = "MADARA_ANALYTICS_PROMETHEUS_ENDPOINT_PORT", long, value_name = "PORT", default_value_t = 9464)]
    pub analytics_prometheus_endpoint_port: u16,
}

impl AnalyticsParams {
    pub fn as_analytics_config(&self) -> AnalyticsConfig {
        AnalyticsConfig {
            service_name: self.analytics_service_name.clone(),
            collection_endpoint: self.analytics_collection_endpoint.clone(),
            prometheus_endpoint_config: PrometheusEndpointConfig {
                enabled: self.analytics_prometheus_endpoint,
                listen_external: self.analytics_prometheus_endpoint_external,
                listen_port: self.analytics_prometheus_endpoint_port,
            },
        }
    }
}
