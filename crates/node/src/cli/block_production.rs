/// Parameters used to config block production.
#[derive(Clone, Debug, clap::Parser)]
pub struct BlockProductionParams {
    /// Disable the block production service.
    /// The block production service is only enabled with the authority (sequencer) mode.
    #[arg(long, alias = "no-block-production")]
    pub block_production_disabled: bool,

    /// Launch in block production mode, with devnet contracts.
    #[arg(long)]
    pub devnet: bool,

    /// Create this number of contracts in the genesis block for the devnet configuration.
    #[arg(long, default_value_t = 10)]
    pub devnet_contracts: u64,
}
