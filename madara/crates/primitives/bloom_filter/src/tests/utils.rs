use std::hash::Hasher;

use crate::{AtomicBitStore, BloomFilter};

// Constants for Bloom filter configuration
// Number of hash functions used in the Bloom filter.
// The value 7 is optimal for a false positive rate of 1% (0.01).
pub const HASH_COUNT: u8 = 7;

// Number of bits per element in the Bloom filter.
// The value 9.6 is optimal for a false positive rate of 1% (0.01).
const BITS_PER_ELEMENT: f64 = 9.6;

pub const FALSE_POSITIF_RATE: f64 = 0.01;

/// Helper function to create a Bloom filter with the given number of elements
pub fn create_filter<H: Hasher + Default>(nb_elem: u64) -> BloomFilter<H, AtomicBitStore> {
    let size = (nb_elem as f64 * BITS_PER_ELEMENT).ceil() as u64;
    BloomFilter::new(size, HASH_COUNT)
}
