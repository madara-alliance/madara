use clap::Args;

/// Parameters used to config prometheus.
#[derive(Debug, Clone, Args)]
pub struct PrometheusParams {
    /// The port used by the prometheus RPC service.
    #[arg(long, value_name = "PORT", default_value = "9615")]
    pub prometheus_port: u16,
    /// Listen on all network interfaces. This usually means the prometheus server will be accessible externally.
    #[arg(long)]
    pub prometheus_external: bool,
    /// Disable the prometheus service.
    #[arg(long)]
    pub no_prometheus: bool,
}
