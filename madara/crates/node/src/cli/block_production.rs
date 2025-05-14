use mc_block_production::BlockProductionMode;
use serde::{Deserialize, Serialize};

/// Parameters used to config block production.
#[derive(Clone, Debug, clap::Parser, Deserialize, Serialize)]
pub struct BlockProductionParams {
    /// Disable the block production service.
    /// The block production service is only enabled with the authority (sequencer) mode.
    #[arg(env = "MADARA_BLOCK_PRODUCTION_DISABLED", long, alias = "no-block-production")]
    pub block_production_disabled: bool,

    /// Mode for block production triggering.
    /// - timed-ticks: Regular interval-based block production (default)
    /// - external-trigger: Only produce blocks when externally triggered
    /// - hybrid: Both timed ticks and external triggers can produce blocks
    #[arg(env = "MADARA_BLOCK_PRODUCTION_MODE", long, conflicts_with = "block_production_disabled", default_value_t = BlockProductionMode::TimedTicks)]
    pub block_production_mode: BlockProductionMode,

    /// Create this number of contracts in the genesis block for the devnet configuration.
    #[arg(env = "MADARA_DEVNET_CONTRACTS", long, default_value_t = 10)]
    pub devnet_contracts: u64,
}
