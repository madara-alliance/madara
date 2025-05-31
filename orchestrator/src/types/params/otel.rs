use crate::cli::instrumentation::InstrumentationCliArgs;
use crate::OrchestratorError;
use url::Url;

#[derive(Debug, Clone)]
pub struct OTELConfig {
    pub endpoint: Option<Url>,
    pub service_name: String,
}

/// from the instrumentation params, we can get the otel config
impl TryFrom<InstrumentationCliArgs> for OTELConfig {
    type Error = OrchestratorError;
    fn try_from(args: InstrumentationCliArgs) -> Result<Self, Self::Error> {
        let service_name = args
            .otel_service_name
            .ok_or_else(|| OrchestratorError::FromDownstreamError("otel_service_name is required".to_string()))?;
        Ok(Self { endpoint: args.otel_collector_endpoint, service_name })
    }
}
