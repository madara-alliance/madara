mod setup_base;
mod setup_madara;

use clap::{Parser, Subcommand};
pub use setup_base::SetupBase;
pub use setup_madara::SetupMadara;

/// Madara Bootstrapper CLI
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct CliArgs {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand, Clone)]
pub enum Commands {
    /// Setup base layer (Ethereum/Starknet)
    SetupBase(SetupBase),

    /// Setup Madara chain
    SetupMadara(SetupMadara),
}
