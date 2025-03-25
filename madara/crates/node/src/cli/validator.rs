use clap::Args;
use mc_submit_tx::TransactionValidatorConfig;

/// Parameters used to config the mempool.
#[derive(Debug, Clone, Args)]
pub struct ValidatorParams {
    /// When enabled, transactions will be validated, even when redirecting the transaction to a gateway.
    /// By default, the validator is only enabled when inserting into the local mempool.
    #[arg(env = "MADARA_ENABLE_TRANSACTION_VALIDATOR", long)]
    pub enable_gateway_redirect_transaction_validator: bool,

    /// Disable transaction validation: no prior validation will be made before inserting into the mempool.
    /// See: Trasaction validation in [Starknet docs Transaction Validation](https://docs.starknet.io/architecture-and-concepts/network-architecture/transaction-life-cycle/)
    #[arg(env = "MADARA_NO_TRANSACTION_VALIDATION", long)]
    pub no_transaction_validation: bool,
}

impl ValidatorParams {
    pub fn as_validator_config(&self) -> TransactionValidatorConfig {
        TransactionValidatorConfig { disable_validation: self.no_transaction_validation }
    }
}
