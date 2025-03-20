use clap::Args;
use mc_mempool::{MempoolConfig, MempoolLimits};
use mp_chain_config::ChainConfig;

/// Parameters used to config the mempool.
#[derive(Debug, Clone, Args)]
pub struct MempoolParams {
    /// Disable mempool validation: no prior validation will be made before inserting into the mempool.
    /// See: Mempool validation in [Starknet docs Transaction Validation](https://docs.starknet.io/architecture-and-concepts/network-architecture/transaction-life-cycle/)
    #[arg(env = "MADARA_NO_MEMPOOL_VALIDATION", long)]
    pub no_mempool_validation: bool,
}

impl MempoolParams {
    pub fn as_mempool_config(&self, chain_config: &ChainConfig) -> MempoolConfig {
        MempoolConfig { limits: MempoolLimits::new(chain_config), disable_validation: self.no_mempool_validation }
    }
}
