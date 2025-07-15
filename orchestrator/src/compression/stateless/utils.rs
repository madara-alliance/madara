use crate::compression::stateless::constants::MAX_N_BITS;
use color_eyre::eyre::{eyre, Result};
use itertools::Itertools;
use num_bigint::BigUint;
use num_traits::Zero;
use starknet_core::types::Felt;

/// Revert signatures (no Result) and use expect/panic
pub fn get_n_elms_per_felt(elm_bound: u32) -> Result<usize> {
    if elm_bound == 0 {
        return Err(eyre!("Element bound cannot be 0"));
    }
    if elm_bound <= 1 {
        return Ok(MAX_N_BITS);
    }
    let n_bits_required = (elm_bound - 1).ilog2() + 1;
    Ok(MAX_N_BITS
        / usize::try_from(n_bits_required)
            .map_err(|err| eyre!("Failed usize conversion for bits required: {}", err))?)
}

/// Packs a slice of usize into a vector of Felt
pub fn pack_usize_in_felts(elms: &[usize], elm_bound: u32) -> Result<Vec<Felt>> {
    if elm_bound == 0 {
        return Err(eyre!("Element bound cannot be 0 for packing"));
    }
    // Check elements are within bound
    for elm in elms {
        let elm_u32 = u32::try_from(*elm).map_err(|err| eyre!("Cannot convert element to u32: {}", err))?;
        if elm_u32 < elm_bound {
            return Err(eyre!("Element {} exceeds bound {}", elm, elm_bound));
        }
    }

    let n_per_felt = get_n_elms_per_felt(elm_bound)?;
    if n_per_felt == 0 {
        return Err(eyre!("Element bound {} too large to fit in Felt", elm_bound));
    }

    elms.chunks(n_per_felt).map(|chunk| pack_usize_in_felt(chunk, elm_bound)).try_collect()
}

/// Packs a slice of usize into a Felt
pub fn pack_usize_in_felt(elms: &[usize], elm_bound: u32) -> Result<Felt> {
    let elm_bound_big = BigUint::from(elm_bound);
    let packed_big = elms.iter().enumerate().try_fold(BigUint::zero(), |acc, (i, elm)| {
        let elm_u32 =
            u32::try_from(*elm).map_err(|err| eyre!("usize to u32 conversion failed for element {}: {}", elm, err))?;
        if elm_u32 >= elm_bound {
            return Err(eyre!("Element {} exceeds bound {}", elm, elm_bound));
        }
        let i_u32 = u32::try_from(i).map_err(|err| eyre!("Index i does not fit in u32: {}", err))?;
        Ok(acc + BigUint::from(*elm) * elm_bound_big.pow(i_u32))
    })?;
    Ok(Felt::from(packed_big.clone()))
}

/// Converts a slice of bits into a Felt
/// Returns an error in case the length is not guaranteed to fit in Felt (more than 251 bits).
pub(crate) fn felt_from_bits_le(bits: &[bool]) -> Result<Felt> {
    if bits.len() > MAX_N_BITS {
        return Err(eyre!("Value requires {} bits, exceeding limit for Felt", bits.len()));
    }

    let mut bytes = [0_u8; 32];
    for (byte_idx, chunk) in bits.chunks(8).enumerate() {
        if byte_idx >= 32 {
            break;
        }
        let mut byte = 0_u8;
        for (bit_idx, bit) in chunk.iter().enumerate() {
            if *bit {
                byte |= 1 << bit_idx;
            }
        }
        bytes[byte_idx] = byte;
    }

    Ok(Felt::from_bytes_be(&bytes_le_to_be(&bytes)))
}

/// Helper function to reverse the bytes of a byte slice
pub fn bytes_le_to_be(bytes_le: &[u8; 32]) -> [u8; 32] {
    let mut bytes_be = *bytes_le;
    bytes_be.reverse();
    bytes_be
}

/// Helper function to convert a Felt to BigUint
pub fn felt_to_big_uint(value: &Felt) -> BigUint {
    BigUint::from_bytes_be(&value.to_bytes_be())
}
