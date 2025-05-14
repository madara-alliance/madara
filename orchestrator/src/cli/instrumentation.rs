use clap::Args;
use url::Url;

/// Parameters used to config instrumentation.
#[derive(Debug, Clone, Args)]
#[group()]
pub struct InstrumentationCliArgs {
    /// The name of the instrumentation service.
    #[arg(env = "MADARA_ORCHESTRATOR_OTEL_SERVICE_NAME", long, default_value = "orchestrator")]
    pub otel_service_name: Option<String>,

    /// The endpoint of the collector.
    #[arg(env = "MADARA_ORCHESTRATOR_OTEL_COLLECTOR_ENDPOINT", long)]
    pub otel_collector_endpoint: Option<Url>,
}
