use clap::Args;
use mc_submit_tx::TransactionValidatorConfig;
use serde::{Deserialize, Serialize};

/// Parameters used to config the mempool.
#[derive(Debug, Clone, Args, Serialize, Deserialize)]
pub struct ValidatorParams {
    /// When enabled, incoming transactions will be validated and then forwarded to the madara-specific validated transaction
    /// gateway. This allows for the separation of the sequencer and gateway (transaction validators) on different machines.
    #[arg(env = "MADARA_VALIDATE_THEN_FORWARD_TXS", long)]
    pub validate_then_forward_txs: bool,

    /// Disable transaction validation: no prior validation will be made before inserting into the mempool.
    /// See: Trasaction validation in [Starknet docs Transaction Validation](https://docs.starknet.io/architecture-and-concepts/network-architecture/transaction-life-cycle/)
    #[arg(env = "MADARA_NO_TRANSACTION_VALIDATION", long)]
    pub no_transaction_validation: bool,

    // TODO: move this, idk where this arg would make sense.
    /// Disable mempool saving. Mempool transactions will not be saved. This can increase performance quite a lot.
    #[arg(env = "MADARA_NO_MEMPOOL_SAVING", long)]
    pub no_mempool_saving: bool,
}

impl ValidatorParams {
    pub fn as_validator_config(&self) -> TransactionValidatorConfig {
        TransactionValidatorConfig { disable_validation: self.no_transaction_validation }
    }
}
