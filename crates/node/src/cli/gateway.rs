use clap::Args;

/// Parameters used to config gateway.
#[derive(Debug, Clone, Args)]
pub struct GatewayParams {
    /// Enable the feeder gateway server.
    #[arg(long, alias = "feeder-gateway")]
    pub feeder_gateway_enable: bool,

    /// Enable the gateway server.
    #[arg(long, alias = "gateway")]
    pub gateway_enable: bool,

    /// Listen on all network interfaces. This usually means the gateway server will be accessible externally.
    #[arg(long)]
    pub gateway_external: bool,

    /// The gateway port to listen at.
    #[arg(long, value_name = "GATEWAY PORT", default_value = "8080")]
    pub gateway_port: u16,
}
