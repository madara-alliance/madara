use sp_runtime::MultiSignature;

/// Alias to 512-bit hash when used in the context of a transaction signature on the chain.
pub type DTxSignatureT = MultiSignature;

/// The type of an index of a transaction in the chain.
pub type DTxIndexT = u128;

/// Deoxys type for the maximum amount of steps allowed for an invoke transaction.
pub type DTxInvokeMaxStepsT = u32;

/// Deoxys type for the maximum amount of steps allowed for validation.
pub type DTxValidateMaxStepsT = u32;
