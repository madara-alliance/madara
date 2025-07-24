use crate::error::job::JobError;
use crate::error::other::OtherError;
use color_eyre::eyre::eyre;
use num_bigint::BigUint;
use num_traits::{One, ToPrimitive, Zero};
use starknet_core::types::Felt;

/// Old format (pre-0.13.3)
///
/// Binary encoding layout (252 bits total):
///
/// Bit positions:
/// 251                                                                                   0
/// |                                                                                     |
/// v                                                                                     v
/// ┌───────────────────────────────────────────────────────┬─────┬─────────┬─────────────┐
/// │                 Zero Padding                          │ cls │  nonce  │ num_changes │
/// │                  (123 bits)                           │ flg │(64 bits)│  (64 bits)  │
/// │                  [251:129]                            │[128]│ [127:64]│   [63:0]    │
/// └───────────────────────────────────────────────────────┴─────┴─────────┴─────────────┘
///
/// Field breakdown:
/// - Zero Padding (123 bits)  : [251:129] - Padded with zeros
/// - class_flag (1 bit)       : [128]     - Class type flag
/// - nonce (64 bits)          : [127:64]  - Nonce value
/// - num_changes (64 bits)    : [63:0]    - Number of changes
///
/// Total used bits: 123 + 1 + 64 + 64 = 252 bits (exactly)
///
pub(super) fn build_da_word_pre_v0_13_3(
    class_flag: bool,
    nonce_change: Option<Felt>,
    num_changes: u64,
) -> Result<BigUint, JobError> {
    let mut da_word = BigUint::zero();

    // Class flag
    da_word |= if class_flag { BigUint::one() << 128 } else { BigUint::zero() };

    // Nonce
    if let Some(new_nonce) = nonce_change {
        let new_nonce = new_nonce
            .to_u64()
            .ok_or_else(|| JobError::Other(OtherError(eyre!("Nonce value {} exceeds u64 maximum", new_nonce))))?;
        da_word |= BigUint::from(new_nonce) << 64;
    }

    // Number of changes
    da_word |= BigUint::from(num_changes);

    Ok(da_word)
}

/// v0.13.3+ format:
///
/// Binary encoding layout (252 bits total):
///
/// Bit positions:
/// 251                                                                                   0
/// |                                                                                     |
/// v                                                                                     v
/// ┌─────────────────────┬────────────────────────┬──────────────────────────────┬───┬───┐
/// │        Zeros        │       new_nonce        │           n_updates          │n_u│cls│
/// │       Padding       │       (64 bits)        │        (8 or 64 bits)        │len│flg│
/// │                     │  [129:66] or [73:10]   │       [65:2] or [9:2]        │[1]│[0]│
/// └─────────────────────┴────────────────────────┴──────────────────────────────┴───┴───┘
///
/// Field breakdown:
/// - new_nonce (64 bits)     : [251:188] - Nonce value
/// - n_updates (variable)    : [187:124] (64-bit) or [187:180] (8-bit, then padded by 0s) - Number of updates
/// - n_updates_len (1 bit)   : [1] - Length indicator (1=8-bit, 0=64-bit n_updates)
/// - class_flag (1 bit)      : [0] - Class changed or not
///
/// Total used bits: 64 + (8|64) + 1 + 1 = 74 or 130 bits
/// Remaining bits: 252 - (74|130) = 178 or 122 bits (reserved/padding)
///
pub(super) fn build_da_word_v0_13_3_and_above(
    class_flag: bool,
    nonce_change: Option<Felt>,
    num_changes: u64,
) -> Result<BigUint, JobError> {
    let mut da_word: BigUint;

    // Determine if we need 8 or 64 bits for num_changes
    let needs_large_updates = num_changes >= 256;

    // Get nonce in 64 bits
    let nonce_felt = nonce_change.unwrap_or(Felt::ZERO);
    let new_nonce = nonce_felt
        .to_u64()
        .ok_or_else(|| JobError::Other(OtherError(eyre!("Nonce value {} exceeds u64 maximum", nonce_felt))))?;

    if needs_large_updates {
        da_word = BigUint::from(new_nonce) << 66;
    } else {
        da_word = (BigUint::from(new_nonce) << 10) | BigUint::from(2u64);
    }

    da_word |= BigUint::from(num_changes) << 2;

    // Class flag
    da_word |= if class_flag { BigUint::one() } else { BigUint::zero() };

    Ok(da_word)
}
