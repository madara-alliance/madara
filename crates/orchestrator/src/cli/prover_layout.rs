use clap::Args;

/// Parameters used to config the server.
#[derive(Debug, Clone, Args)]
#[group()]
pub struct ProverLayoutCliArgs {
    /// The layout name for SNOS.
    #[arg(env = "MADARA_ORCHESTRATOR_SNOS_LAYOUT_NAME", long, default_value = "all_cairo")]
    pub snos_layout_name: String,

    /// The layout name for the prover.
    #[arg(env = "MADARA_ORCHESTRATOR_PROVER_LAYOUT_NAME", long, default_value = "dynamic")]
    pub prover_layout_name: String,
}
