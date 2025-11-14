use serde::{Deserialize, Serialize};

/// Parameters used to config block production.
#[derive(Clone, Debug, clap::Parser, Deserialize, Serialize)]
pub struct BlockProductionParams {
    /// Disable the block production service.
    /// The block production service is only enabled with the authority (sequencer) mode.
    #[arg(env = "MADARA_BLOCK_PRODUCTION_DISABLED", long, alias = "no-block-production")]
    pub block_production_disabled: bool,

    /// Create this number of contracts in the genesis block for the devnet configuration.
    #[arg(env = "MADARA_DEVNET_CONTRACTS", long, default_value_t = 10)]
    pub devnet_contracts: u64,

    /// Close preconfirmed block on restart instead of continuing with it.
    /// When false (default), the node will resume with the preconfirmed block after restart.
    /// When true, the preconfirmed block will be closed (finalized) on startup.
    #[arg(env = "MADARA_CLOSE_PRECONFIRMED_BLOCK_UPON_RESTART", long)]
    pub close_preconfirmed_block_upon_restart: bool,
}
