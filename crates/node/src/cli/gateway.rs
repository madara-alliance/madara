use clap::Args;
use serde::Deserialize;

/// Parameters used to config gateway.
#[derive(Debug, Clone, Args, Deserialize)]
#[serde(default)]
pub struct GatewayParams {
    /// Enable the feeder gateway server.
    #[arg(env = "MADARA_FEEDER_GATEWAY_ENABLE", long, alias = "feeder-gateway")]
    pub feeder_gateway_enable: bool,

    /// Enable the gateway server.
    #[arg(env = "MADARA_GATEWAY_ENABLE", long, alias = "gateway")]
    pub gateway_enable: bool,

    /// Listen on all network interfaces. This usually means the gateway server will be accessible externally.
    #[arg(env = "MADARA_GATEWAY_EXTERNAL", long)]
    pub gateway_external: bool,

    /// The gateway port to listen at.
    #[arg(env = "MADARA_GATEWAY_PORT", long, value_name = "GATEWAY PORT", default_value_t = 8080)]
    pub gateway_port: u16,
}

impl Default for GatewayParams {
    fn default() -> Self {
        Self {
            feeder_gateway_enable: false,
            gateway_enable: false,
            gateway_external: false,
            gateway_port: 8080,
        }
    }
}
