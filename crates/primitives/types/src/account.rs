use sp_runtime::traits::{IdentifyAccount, Verify};

use crate::transactions::DTxSignatureT;

/// Some way of identifying an account on the chain. We intentionally make it equivalent
/// to the public key of our transaction signing scheme.
pub type DAccountIdT = <<DTxSignatureT as Verify>::Signer as IdentifyAccount>::AccountId;

/// Type used for the balance of an account.
pub type DAccountBalanceT = u128;
