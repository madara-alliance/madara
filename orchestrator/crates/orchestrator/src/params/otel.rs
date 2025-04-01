use url::Url;
use crate::cli::instrumentation::InstrumentationCliArgs;
use crate::{OrchestratorError};

#[derive(Debug, Clone)]
pub struct OTELConfig {
    pub endpoint: Url,
    pub service_name: String,
}


/// from the instrumentation params, we can get the otel config
impl TryFrom<InstrumentationCliArgs> for OTELConfig {
    type Error = OrchestratorError;
    fn try_from(args: InstrumentationCliArgs) -> Result<Self, Self::Error> {
        let endpoint = args.otel_collector_endpoint.clone().ok_or_else(|e| OrchestratorError::FromDownstreamError(e))?;
        let service_name = args.otel_service_name.clone().ok_or_else(|e| OrchestratorError::FromDownstreamError(e))?;
        Ok(Self {
            endpoint,
            service_name,
        })
    }
}

impl From<InstrumentationCliArgs> for OTELConfig {
    fn from(args: InstrumentationCliArgs) -> Self {
        Self {
            endpoint: args.otel_collector_endpoint.unwrap(),
            service_name: args.otel_service_name.unwrap(),
        }
    }
}