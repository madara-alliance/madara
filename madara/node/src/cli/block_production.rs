use serde::{Deserialize, Serialize};

/// Parameters used to config block production.
#[derive(Clone, Debug, clap::Parser, Deserialize, Serialize)]
pub struct BlockProductionParams {
    /// Disable the block production service.
    /// The block production service is only enabled with the authority (sequencer) mode.
    #[arg(env = "MADARA_BLOCK_PRODUCTION_DISABLED", long, alias = "no-block-production")]
    pub block_production_disabled: bool,

    /// Discard any open preconfirmed block on sequencer startup instead of re-executing and closing it.
    /// This permanently drops the preconfirmed transactions: they are neither replayed nor re-queued.
    /// The current runtime execution config is then persisted for subsequent block production.
    #[serde(default)]
    #[arg(env = "MADARA_DISCARD_PRECONFIRMED_ON_STARTUP", long)]
    pub discard_preconfirmed_on_startup: bool,

    /// Create this number of contracts in the genesis block for the devnet configuration.
    #[arg(env = "MADARA_DEVNET_CONTRACTS", long, default_value_t = 10)]
    pub devnet_contracts: u64,

    /// Initialize devnet through the storage-proof-compatible bootstrap path.
    ///
    /// This is an opt-in path for slower, proof-oriented devnet setup work. The default devnet
    /// bootstrap remains optimized for local development speed.
    #[serde(default)]
    #[arg(env = "MADARA_DEVNET_STORAGE_PROOF_BOOTSTRAP", long, requires = "devnet")]
    pub devnet_storage_proof_bootstrap: bool,
}
