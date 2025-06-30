use clap::Args;

#[derive(Debug, Clone, Args)]
pub struct SetupMadara {
    /// Path to the configuration file
    #[arg(long)]
    pub config_path: String,

    /// Path to the base layer addresses file
    #[arg(long)]
    pub base_addresses_path: String,

    /// Path to output the Madara deployment addresses
    #[arg(long)]
    pub output_path: String,

    /// Private key for Madara deployment (from environment variable)
    #[arg(long, env = "MADARA_PRIVATE_KEY")]
    pub private_key: String,

    /// Base layer private key (from environment variable)
    #[arg(long, env = "BASE_LAYER_PRIVATE_KEY")]
    pub base_layer_private_key: String,
}
