use clap::Args;
use serde::{Deserialize, Serialize};

/// The default port.
pub const FGW_DEFAULT_PORT: u16 = 8080;

/// Parameters used to config gateway.
#[derive(Debug, Clone, Args, Deserialize, Serialize)]
pub struct GatewayParams {
    /// Enable the feeder gateway server.
    #[arg(env = "MADARA_FEEDER_GATEWAY_ENABLE", long)]
    pub feeder_gateway_enable: bool,

    /// Enable the gateway server.
    #[arg(env = "MADARA_GATEWAY_ENABLE", long)]
    pub gateway_enable: bool,

    /// Listen on all network interfaces. This usually means the gateway server will be accessible externally.
    #[arg(env = "MADARA_GATEWAY_EXTERNAL", long)]
    pub gateway_external: bool,

    /// The gateway port to listen at.
    #[arg(env = "MADARA_GATEWAY_PORT", long, value_name = "GATEWAY PORT", default_value_t = FGW_DEFAULT_PORT)]
    pub gateway_port: u16,
}
