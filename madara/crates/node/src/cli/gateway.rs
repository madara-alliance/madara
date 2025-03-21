use clap::Args;
use mc_gateway_server::service::GatewayServerConfig;

/// The default port.
pub const FGW_DEFAULT_PORT: u16 = 8080;

/// Parameters used to config gateway.
#[derive(Debug, Clone, Args)]
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

    /// Enable the madara-specific add_verified_transaction. Beware that this endpoint should not be exposed, this
    /// is for internal trusted madara-to-madara communication only.
    #[arg(env = "GATEWAY_MADARA_ENABLE_TRUSTED_ADD_VERIFIED_TRANSACTION", long)]
    pub gateway_enable_trusted_add_verified_transaction: bool,

    /// The gateway port to listen on.
    #[arg(env = "MADARA_GATEWAY_PORT", long, value_name = "PORT", default_value_t = FGW_DEFAULT_PORT)]
    pub gateway_port: u16,
}

impl GatewayParams {
    pub fn as_gateway_server_config(&self) -> GatewayServerConfig {
        GatewayServerConfig {
            feeder_gateway_enable: self.feeder_gateway_enable,
            gateway_enable: self.gateway_enable,
            gateway_external: self.gateway_external,
            gateway_port: self.gateway_port,
            enable_trusted_add_verified_transaction: self.gateway_enable_trusted_add_verified_transaction,
        }
    }
}
