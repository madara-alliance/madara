use crate::compression::stateless::constants::MAX_N_BITS;
use crate::compression::stateless::utils::felt_from_bits_le_bytes_be;
use color_eyre::eyre::{bail, eyre};
use starknet_core::types::Felt;
use std::any::type_name;
use std::cmp::max;
use strum_macros::Display;

// BitsArray
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct BitsArray<const LENGTH: usize>(pub(crate) [bool; LENGTH]);

// Implementing TryFrom<Felt> for BitsArray<LENGTH>
impl<const LENGTH: usize> TryFrom<Felt> for BitsArray<LENGTH> {
    type Error = color_eyre::Report;

    fn try_from(felt: Felt) -> color_eyre::Result<Self, Self::Error> {
        let n_bits_felt = felt.bits();
        if n_bits_felt > LENGTH {
            // Special case for Felt::ZERO
            if felt == Felt::ZERO && LENGTH >= 1 {
                // Allow zero if LENGTH is enough
            } else {
                bail!(
                    "Value {} requires {} bits, exceeding limit {} for BitsArray<{}>",
                    felt,
                    n_bits_felt,
                    LENGTH,
                    LENGTH
                );
            }
        }
        // Original used felt.to_bits_le()[0..LENGTH]. Let's stick to a BigUint method if to_bits_le isn't ideal/available
        let felt_as_biguint = felt.to_biguint();
        let mut bits_vec = Vec::with_capacity(LENGTH);
        for i in 0..LENGTH {
            bits_vec.push(felt_as_biguint.bit(i as u64));
        }
        let bits_array = bits_vec
            .try_into()
            .map_err(|v: Vec<bool>| eyre!("Failed to convert vec of len {} to array of len {}", v.len(), LENGTH))?;
        Ok(Self(bits_array))
    }
}

// Implementing TryFrom<BitsArray<LENGTH>> for Felt
impl<const LENGTH: usize> TryFrom<BitsArray<LENGTH>> for Felt {
    type Error = color_eyre::Report;

    fn try_from(bits_array: BitsArray<LENGTH>) -> color_eyre::Result<Self, Self::Error> {
        felt_from_bits_le_bytes_be(&bits_array.0)
    }
}

// BitLength enum
// This is used for dividing the uncompressed state update data into buckets for compression
#[derive(Debug, Display, strum_macros::EnumCount, Clone, Copy)]
pub(crate) enum BitLength {
    Bits15,
    Bits31,
    Bits62,
    Bits83,
    Bits125,
    Bits252,
}

impl BitLength {
    pub(crate) const fn n_bits(&self) -> usize {
        match self {
            Self::Bits15 => 15,
            Self::Bits31 => 31,
            Self::Bits62 => 62,
            Self::Bits83 => 83,
            Self::Bits125 => 125,
            Self::Bits252 => 252,
        }
    }

    pub(crate) fn n_elems_in_felt(&self) -> usize {
        max(MAX_N_BITS / self.n_bits(), 1)
    }

    // Use usize consistent with Felt::bits()
    pub(crate) fn min_bit_length(n_bits: usize) -> color_eyre::Result<Self> {
        match n_bits {
            0 => Ok(Self::Bits15), // Handle 0 bits case explicitly if needed, mapping to Bits15
            _ if n_bits <= 15 => Ok(Self::Bits15),
            _ if n_bits <= 31 => Ok(Self::Bits31),
            _ if n_bits <= 62 => Ok(Self::Bits62),
            _ if n_bits <= 83 => Ok(Self::Bits83),
            _ if n_bits <= 125 => Ok(Self::Bits125),
            _ if n_bits <= 252 => Ok(Self::Bits252),
            _ => bail!("Value requires {} bits, exceeding limit for {}", n_bits, type_name::<Self>()),
        }
    }
}
