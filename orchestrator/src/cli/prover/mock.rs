use clap::Args;

/// Parameters used to config the Mock prover.
///
/// The mock prover is for mocknet / dev environments. It does not submit proofs anywhere;
/// when `MADARA_ORCHESTRATOR_MOCK_VERIFIER_ADDRESS` is set, it registers the aggregator
/// fact hash on that L1 `MockGpsVerifier` contract so the Starknet core contract's
/// `isValid(fact)` check passes during state-transition. When unset, registration is
/// skipped (assumes the settlement core contract points at an always-true verifier stub).
///
/// Ethereum RPC + private key are reused from the Ethereum settlement config.
///
/// Selection of this prover is now via the top-level `--prover` arg /
/// `MADARA_ORCHESTRATOR_PROVER=mock` env. There is no `--mock` flag.
#[derive(Debug, Clone, Args)]
pub struct MockCliArgs {
    /// Optional L1 address of a deployed `MockGpsVerifier`. When set, the mock prover sends
    /// `registerFact(bytes32)` txs during aggregator processing.
    #[arg(env = "MADARA_ORCHESTRATOR_MOCK_VERIFIER_ADDRESS", long)]
    pub mock_verifier_address: Option<String>,
}
