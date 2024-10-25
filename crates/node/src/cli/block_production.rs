/// Parameters used to config block production.
#[derive(Clone, Debug, clap::Parser)]
pub struct BlockProductionParams {
    /// Disable the block production service.
    /// The block production service is only enabled with the authority (sequencer) mode.
    #[arg(env = "MADARA_BLOCK_PRODUCTION_DISABLED", long, alias = "no-block-production")]
    pub block_production_disabled: bool,

    /// Launch a devnet with a production chain id (like SN_MAINNET, SN_SEPOLIA).
    /// This in unsafe because your devnet transactions can be replayed on the actual network.
    #[arg(env = "MADARA_OVERRIDE_DEVNET_CHAIN_ID", long, default_value_t = false)]
    pub override_devnet_chain_id: bool,

    /// Create this number of contracts in the genesis block for the devnet configuration.
    #[arg(env = "MADARA_DEVNET_CONTRACTS", long, default_value_t = 10)]
    pub devnet_contracts: u64,
}
