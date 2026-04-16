pub const SHARP_PROGRAM_HASH_FUNCTION: &str = "PEDERSEN";

/// Child proof-creation jobs always use offchain proof.
/// Core contract verifies Applicative job's proof.
pub const CHILD_JOB_OFFCHAIN_PROOF: &str = "true";
/// Applicative (aggregator) jobs: proof must be registered on-chain so the core contract
/// sees `isValid(fact) == true` during state update. Hence offchain = false.
pub const APPLICATIVE_JOB_OFFCHAIN_PROOF: &str = "false";
