use clap::Args;

/// Parameters used to config telemetry.
#[derive(Debug, Clone, Args)]
pub struct SetupBase {
    /// Path to the configuration file
    #[arg(long)]
    pub config_path: String,

    /// Path to output the deployed addresses (JSON)
    #[arg(long)]
    pub addresses_output_path: String,

    /// Private key for deployment (from environment variable)
    #[arg(long, env = "BASE_LAYER_PRIVATE_KEY")]
    pub private_key: String,
}
