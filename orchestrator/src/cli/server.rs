use clap::Args;

/// Parameters used to config the server.
#[derive(Debug, Clone, Args)]
#[group()]
pub struct ServerCliArgs {
    /// The host to listen on.
    #[arg(env = "MADARA_ORCHESTRATOR_HOST", long, default_value = "127.0.0.1")]
    pub host: String,

    /// The port to listen on.
    #[arg(env = "MADARA_ORCHESTRATOR_PORT", long, default_value = "3000")]
    pub port: u16,

    /// Enable admin endpoints for privileged operations.
    #[arg(env = "MADARA_ORCHESTRATOR_ADMIN_ENABLED", long, default_value_t = false)]
    pub admin_enabled: bool,
}
