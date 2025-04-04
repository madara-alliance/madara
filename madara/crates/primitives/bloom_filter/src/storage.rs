//! Storage backends for Bloom filters.
//!
//! This module provides both atomic (thread-safe) and non-atomic storage implementations
//! for the bit array used by the Bloom filter.

use serde::{de::Visitor, Deserialize, Deserializer, Serialize, Serializer};
use std::{
    fmt,
    sync::atomic::{AtomicU64, Ordering},
};

/// An immutable bit storage implementation using regular u64 values.
#[derive(Serialize, Deserialize, Clone)]
#[serde(transparent)]
#[repr(transparent)]
pub struct BitStore {
    bits: Box<[u64]>,
}

/// A thread-safe bit storage implementation using atomic u64 values.
#[repr(transparent)]
pub struct AtomicBitStore {
    bits: Box<[AtomicU64]>,
}

#[cfg(test)]
impl Clone for AtomicBitStore {
    fn clone(&self) -> Self {
        let cloned_bits = self
            .bits
            .iter()
            .map(|atomic| {
                let value = atomic.load(std::sync::atomic::Ordering::Relaxed);
                std::sync::atomic::AtomicU64::new(value)
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();

        Self { bits: cloned_bits }
    }
}

/// Custom serialization for AtomicBitStore that converts atomic values to regular u64s.
impl Serialize for AtomicBitStore {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let regular_bits: Vec<u64> = self.bits.iter().map(|atomic| atomic.load(Ordering::Relaxed)).collect();
        regular_bits.serialize(serializer)
    }
}

/// Custom visitor for deserializing AtomicBitStore from regular u64 values.
struct AtomicBitStoreVisitor;

impl<'de> Visitor<'de> for AtomicBitStoreVisitor {
    type Value = AtomicBitStore;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("an array of unsigned 64-bit integers")
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        let mut regular_bits = Vec::new();
        while let Some(value) = seq.next_element()? {
            regular_bits.push(value);
        }

        let atomic_bits = regular_bits.into_iter().map(AtomicU64::new).collect();

        Ok(AtomicBitStore { bits: atomic_bits })
    }
}

impl<'de> Deserialize<'de> for AtomicBitStore {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(AtomicBitStoreVisitor)
    }
}

impl BitStore {
    /// Creates a new BitStore from a boxed slice of u64 values.
    #[inline]
    pub(crate) fn new_from_slice(bits: Box<[u64]>) -> Self {
        Self { bits }
    }

    /// Returns the total number of bits in the store.
    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.bits.len() << 6
    }

    /// Counts the total number of set bits (ones) in the store.
    #[inline]
    pub(crate) fn count_ones(&self) -> usize {
        self.bits.iter().map(|&block| block.count_ones() as usize).sum()
    }

    /// Tests whether a specific bit is set.
    ///
    /// # Arguments
    ///
    /// * `index`: The bit position to test
    ///
    /// # Implementation Notes
    ///
    /// The function uses direct indexing since input validation is performed at the hash
    /// generation level, making additional bounds checking redundant.
    #[inline]
    pub(crate) fn test_bit(&self, index: usize) -> bool {
        let (block_index, bit_mask) = bit_position(index);
        self.bits[block_index] & bit_mask != 0
    }

    #[cfg(test)]
    pub(crate) fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};

        let mut hasher = std::collections::hash_map::DefaultHasher::default();
        for b in &self.bits {
            b.hash(&mut hasher);
        }
        hasher.finish()
    }
}

impl AtomicBitStore {
    /// Creates a new AtomicBitStore with the specified size in u64 blocks.
    pub(crate) fn new(size_in_u64s: usize) -> Self {
        let bits = (0..size_in_u64s).map(|_| AtomicU64::new(0)).collect();
        Self { bits }
    }

    /// Sets a specific bit to 1.
    ///
    /// # Arguments
    ///
    /// * `index`: The bit position to set
    ///
    /// # Implementation Notes
    ///
    /// The function uses direct indexing since input validation is performed at the hash
    /// generation level, making additional bounds checking redundant.
    #[inline]
    pub(crate) fn set_bit(&self, index: usize) {
        let (block_index, bit_mask) = bit_position(index);
        self.bits[block_index].fetch_or(bit_mask, Ordering::Relaxed);
    }

    /// Converts this atomic store into a regular BitStore.
    pub(crate) fn to_readonly(&self) -> BitStore {
        let bits = self.bits.iter().map(|b| b.load(Ordering::Relaxed)).collect();
        BitStore::new_from_slice(bits)
    }

    #[cfg(test)]
    pub(crate) fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};

        let mut hasher = std::collections::hash_map::DefaultHasher::default();
        for b in &self.bits {
            b.load(Ordering::Relaxed).hash(&mut hasher);
        }
        hasher.finish()
    }
}

/// Calculates the block index and bit mask for a given bit position.
///
/// The bit position is split into two components:
/// - A block index identifying which u64 block contains the bit
/// - A bit mask with a single 1 at the target position within that block
///
/// # Arguments
///
/// * `index`: Global bit position in the bitset
///
/// # Returns
///
/// A tuple containing:
/// * `block_index`: Index of the u64 block (index >> 6 is equivalent to index / 64)
/// * `bit_mask`: A u64 with a single bit set (1 << position_in_block)
///
/// # Implementation Notes
///
/// - Uses bit shifts for efficient division/modulo:
///   - `>> 6` divides by 64 to get block index
///   - `& 0x3F` is equivalent to modulo 64 for bit position
/// - Operations are const and will be computed at compile time when possible
#[inline(always)]
const fn bit_position(index: usize) -> (usize, u64) {
    // Calculate which u64 block contains our bit
    let block_index = index >> 6;
    // Create a mask with a 1 bit in the correct position (0-63) within the block
    let bit_mask = 1u64 << (index & 0x3F);
    (block_index, bit_mask)
}
