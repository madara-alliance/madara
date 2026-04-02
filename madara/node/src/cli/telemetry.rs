use clap::{ArgAction, Args};
use mc_telemetry::TelemetryConfig;
use mp_utils::parsers::parse_url;
use serde::{Deserialize, Serialize};
use url::Url;

fn default_service_name() -> String {
    "madara".into()
}

fn default_metrics_export() -> bool {
    true
}

/// Parameters used to configure OpenTelemetry.
#[derive(Debug, Clone, Args, Deserialize, Serialize)]
pub struct TelemetryParams {
    /// Name of the OTEL service.
    #[arg(env = "MADARA_OTEL_SERVICE_NAME", long, default_value = default_service_name())]
    #[serde(default = "default_service_name")]
    pub otel_service_name: String,

    /// Endpoint of the OTEL collector.
    #[arg(env = "OTEL_EXPORTER_OTLP_ENDPOINT", long, value_parser = parse_url)]
    #[serde(default)]
    pub otel_collector_endpoint: Option<Url>,

    /// Export metrics to the OTEL collector.
    #[arg(env = "OTEL_EXPORT_METRICS", long, action = ArgAction::Set, default_value_t = default_metrics_export())]
    #[serde(default = "default_metrics_export")]
    pub otel_export_metrics: bool,

    /// Export traces to the OTEL collector.
    #[arg(env = "OTEL_EXPORT_TRACES", long)]
    #[serde(default)]
    pub otel_export_traces: bool,

    /// Export logs to the OTEL collector.
    #[arg(env = "OTEL_EXPORT_LOGS", long)]
    #[serde(default)]
    pub otel_export_logs: bool,
}

impl TelemetryParams {
    pub fn as_telemetry_config(&self) -> TelemetryConfig {
        TelemetryConfig {
            service_name: self.otel_service_name.clone(),
            collection_endpoint: self.otel_collector_endpoint.clone(),
            export_metrics: self.otel_export_metrics,
            export_traces: self.otel_export_traces,
            export_logs: self.otel_export_logs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::RunCmd;
    use clap::Parser;

    #[test]
    fn telemetry_params_use_new_defaults() {
        let run_cmd = RunCmd::parse_from(["madara", "--full", "--network", "sepolia"]);

        assert_eq!(run_cmd.telemetry_params.otel_service_name, "madara");
        assert_eq!(run_cmd.telemetry_params.otel_collector_endpoint, None);
        assert!(run_cmd.telemetry_params.otel_export_metrics);
        assert!(!run_cmd.telemetry_params.otel_export_traces);
        assert!(!run_cmd.telemetry_params.otel_export_logs);
    }

    #[test]
    fn telemetry_params_parse_explicit_otel_flags() {
        let run_cmd = RunCmd::parse_from([
            "madara",
            "--full",
            "--network",
            "sepolia",
            "--otel-service-name",
            "custom-node",
            "--otel-collector-endpoint",
            "http://127.0.0.1:4317",
            "--otel-export-metrics",
            "false",
            "--otel-export-traces",
            "--otel-export-logs",
        ]);

        assert_eq!(run_cmd.telemetry_params.otel_service_name, "custom-node");
        assert_eq!(
            run_cmd.telemetry_params.otel_collector_endpoint.as_ref().map(Url::as_str),
            Some("http://127.0.0.1:4317/")
        );
        assert!(!run_cmd.telemetry_params.otel_export_metrics);
        assert!(run_cmd.telemetry_params.otel_export_traces);
        assert!(run_cmd.telemetry_params.otel_export_logs);
    }
}
