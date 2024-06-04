use clap::Args;

/// Parameters used to config prometheus.
#[derive(Debug, Clone, Args)]
pub struct PrometheusParams {
	/// Specify Prometheus exporter TCP Port.
	#[arg(long, value_name = "PORT")]
	pub prometheus_port: Option<u16>,
	/// Expose Prometheus exporter on all interfaces.
	/// Default is local.
	#[arg(long)]
	pub prometheus_external: bool,
	/// Do not expose a Prometheus exporter endpoint.
	/// Prometheus metric endpoint is enabled by default.
	#[arg(long)]
	pub no_prometheus: bool,
}
