//! Bloom filter implementation with configurable hash functions and storage backends.
//!
//! A Bloom filter is a space-efficient probabilistic data structure used to test whether an element
//! is a member of a set. False positive matches are possible, but false negatives are not. In other
//! words, a query returns either "possibly in set" or "definitely not in set."
//!
//! # Features
//!
//! - Generic over hash functions
//! - Supports both thread-safe (atomic) and single-threaded storage backends
//! - Pre-calculated hash optimization for repeated queries
//! - Serialization support via serde

use crate::storage::{AtomicBitStore, BitStore};
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
/// A cache of computed hash positions that can be reused across different filter sizes.
///
/// This structure stores the raw hash values generated for an item, allowing them to be
/// efficiently mapped to different filter sizes without recalculating the hash functions.
/// This is particularly useful when the same item needs to be checked against multiple
/// Bloom filters of different sizes.
///
/// # Requirements
///
/// The number of hash functions used to create this pre-calculated hash set must match
/// the number of hash functions used by any Bloom filter it's used with. Attempting to
/// use these hashes with a filter having a different hash count will result in incorrect
/// behavior.
#[derive(Debug, Clone)]
pub struct PreCalculatedHashes {
    /// Raw hash values before modulo operation
    raw_hashes: Vec<u64>,
    /// Number of hash functions used
    hash_count: u8,
}

impl PreCalculatedHashes {
    /// Creates a new instance with pre-calculated hash values for the given item.
    ///
    /// # Type Parameters
    ///
    /// * `H`: The hasher implementation to use (must implement `Hasher + Default`)
    /// * `T`: The type of item being hashed (must implement `Hash`)
    ///
    /// # Parameters
    ///
    /// * `hash_count`: Number of hash functions to use (must match the target filter's hash count)
    /// * `item`: Reference to the item to generate hashes for
    ///
    /// # Returns
    ///
    /// A new `PreCalculatedHashes` instance containing the computed hash values
    pub fn new<H: Hasher + Default, T: Hash>(hash_count: u8, item: &T) -> Self {
        Self { raw_hashes: calculate_hashes::<H, T>(hash_count, item).collect(), hash_count }
    }

    /// Returns the number of hash functions used in these pre-calculations.
    ///
    /// This value represents how many different bit positions will be generated
    /// for each filter size and must match the target filter's hash count.
    #[inline]
    pub fn hash_count(&self) -> u8 {
        self.hash_count
    }

    /// Returns a reference to the raw hash values before modulo operation.
    ///
    /// These values are the unmodified results from the hash functions,
    /// before being mapped to specific filter sizes.
    #[inline]
    pub fn raw_hashes(&self) -> &[u64] {
        &self.raw_hashes
    }

    /// Calculates bit positions for a specific filter size using the pre-calculated hashes.
    ///
    /// This method maps the raw hash values to actual bit positions within a filter
    /// of the specified size using modulo operation.
    ///
    /// # Parameters
    ///
    /// * `filter_size`: The size of the target Bloom filter in bits
    ///
    /// # Returns
    ///
    /// An iterator yielding `hash_count` bit positions, each in the range [0, filter_size)
    ///
    /// # Notes
    ///
    /// - The returned positions are guaranteed to be within the valid range for the
    ///   specified filter size, but may not be unique if collisions occur.
    /// - The target Bloom filter must have the same number of hash functions as was used
    ///   to create these pre-calculated hashes. Using these positions with a filter that
    ///   has a different hash count will lead to incorrect behavior.
    #[inline]
    pub fn get_positions_for_size(&self, filter_size: u64) -> impl Iterator<Item = usize> + '_ {
        self.raw_hashes.iter().map(move |hash| (hash % filter_size) as usize)
    }
}

/// A Bloom filter implementation with configurable hash function and storage backend.
///
/// # Type Parameters
///
/// * `H`: The hasher implementation to use for generating hash values
/// * `B`: The storage backend type (either `AtomicBitStore` for mutable or `BitStore` for immutable)
///
/// # Fields
///
/// * `storage`: The bit array storage backend
/// * `bit_size`: The total number of bits in the filter
/// * `hash_count`: The number of hash functions used
/// * `_hasher`: Phantom data for the hasher type
#[derive(Serialize, Deserialize, Clone)]
pub struct BloomFilter<H: Hasher + Default, B> {
    storage: B,
    bit_size: u64,
    hash_count: u8,
    #[serde(skip)]
    _hasher: PhantomData<H>,
}

impl<H: Hasher + Default> BloomFilter<H, AtomicBitStore> {
    /// Creates a new Bloom filter with the specified size and number of hash functions.
    ///
    /// The actual size will be aligned to the next multiple of 64 bits for efficient storage.
    ///
    /// # Arguments
    ///
    /// * `size`: The desired size of the filter in bits
    /// * `hash_count`: The number of hash functions to use
    pub fn new(size: u64, hash_count: u8) -> Self {
        let size_in_u64s = (size.max(1) + 63) >> 6;
        let aligned_size = size_in_u64s << 6;

        Self {
            storage: AtomicBitStore::new(size_in_u64s as _),
            bit_size: aligned_size,
            hash_count,
            _hasher: PhantomData,
        }
    }

    /// Converts this mutable Bloom filter into an immutable one.
    ///
    /// This is useful when you're done adding elements and want to switch to a read-only version.
    pub fn finalize(self) -> BloomFilter<H, BitStore> {
        BloomFilter {
            storage: self.storage.to_readonly(),
            bit_size: self.bit_size,
            hash_count: self.hash_count,
            _hasher: PhantomData,
        }
    }

    /// Adds an item to the Bloom filter.
    ///
    /// # Arguments
    ///
    /// * `item`: The item to add, which must implement the Hash trait
    pub fn add<T: Hash>(&self, item: &T) {
        calculate_hashes::<H, T>(self.hash_count, item)
            .for_each(|hash| self.storage.set_bit((hash % self.bit_size) as usize));
    }
}

impl<H: Hasher + Default, B> BloomFilter<H, B> {
    /// Returns the size of the Bloom filter in bits.
    pub fn len(&self) -> u64 {
        self.bit_size
    }

    /// Returns the number of hash functions used by the filter.
    pub fn hash_count(&self) -> u8 {
        self.hash_count
    }

    pub(crate) fn storage(&self) -> &B {
        &self.storage
    }
}

impl<H: Hasher + Default> BloomFilter<H, BitStore> {
    /// Creates a new immutable Bloom filter from a bit storage backend.
    pub fn from_storage(storage: BitStore, hash_count: u8) -> Self {
        let bit_size = storage.len() as _;
        Self { storage, bit_size, hash_count, _hasher: PhantomData }
    }

    /// Tests whether an item might be in the set (immutable version).
    pub fn might_contain<T: Hash>(&self, item: &T) -> bool {
        calculate_hashes::<H, T>(self.hash_count, item)
            .all(|hash| self.storage.test_bit((hash % self.bit_size) as usize))
    }

    /// Tests whether an item might be in the set using pre-calculated hash values.
    pub fn might_contain_hashes(&self, pre_calculated: &PreCalculatedHashes) -> bool {
        pre_calculated.get_positions_for_size(self.bit_size).all(|pos| self.storage.test_bit(pos))
    }

    /// Estimates the current false positive rate of the filter based on
    /// the number of bits set and the number of hash functions.
    pub fn estimated_false_positive_rate(&self) -> f64 {
        let set_bits = self.storage.count_ones() as f64;
        let total_bits = self.storage.len() as f64;
        (set_bits / total_bits).powi(self.hash_count as i32)
    }
}

/// Calculates multiple hash positions for an item using double hashing technique.
///
/// This function implements a double hashing strategy to generate k different bit
/// positions using the formula:
///
/// h(i) = (h1 + i * h2) mod m
///
/// where:
/// - h1 is the primary hash value of the item
/// - h2 is a secondary hash derived from h1 using rotation and golden ratio multiplication
/// - m is the bit array size (applied implicitly by the caller)
/// - i iterates from 0 to k-1, with k being the number of desired hash functions
///
/// # Mathematical Justification
///
/// ## Double Hashing
/// - Double hashing is used to minimize clustering in Bloom filters.
/// - The choice of a secondary hash function (`h2`) prevents patterns from forming in hash placement.
/// - It ensures an **even and uniform spread** of bits, reducing collisions.
/// - Reference: [Kirsch & Mitzenmacher (2006) - "Less Hashing, Same Performance"](https://www.eecs.harvard.edu/~michaelm/postscripts/rsa2008.pdf)
///
/// ## Golden Ratio Constant (`0x9e37_79b9_7f4a_7c15`)
/// The function uses **PHI = 2^64/φ**, where `φ` is the golden ratio (~1.6180339887). This value is chosen because:
/// - It provides **optimal distribution properties** in hashing.
/// - When used in multiplication, it ensures **good bit avalanche effects**.
/// - Its **binary representation distributes bits uniformly**, preventing poor hash results.
/// - It minimizes **correlation between input and output bits** in modular arithmetic.
/// - Reference: [Knuth, D. (1998). "The Art of Computer Programming, Volume 3"](https://www-cs-faculty.stanford.edu/~knuth/taocp.html)
///
/// ## Bit Rotation (`17`)
/// The **17-bit rotation** was selected because:
/// - It is **coprime with 64** (word size), ensuring maximum period length.
/// - Provides **effective bit mixing** while remaining computationally efficient.
/// - As an **odd number**, it avoids potential alignment patterns from even rotations.
/// - Creates a **balanced distribution** between upper and lower word bits.
///
/// The combination of **golden ratio multiplication** and **17-bit rotation** ensures **strong avalanche properties**
/// and **computational efficiency**.
///
///
/// # Parameters
///
/// * `hash_count`: Number of hash positions to generate
/// * `item`: Reference to the item to be hashed
///
/// # Type Parameters
///
/// * `H`: A hasher type that implements `Hasher` + `Default`
/// * `T`: The type of item being hashed, must implement `Hash`
///
/// # Returns
///
/// Returns an iterator yielding `hash_count` different hash values. Each value
/// represents a position that can be mapped to the underlying bit array through
/// modulo operation by the caller.
///
/// Note: The actual bit positions should be computed by the caller by applying
/// modulo with the bit array size to the returned hash values.
#[inline]
fn calculate_hashes<H: Hasher + Default, T: Hash>(hash_count: u8, item: &T) -> impl Iterator<Item = u64> + '_ {
    let mut hasher1 = H::default();
    item.hash(&mut hasher1);
    let h1 = hasher1.finish();

    const PHI: u64 = 0x9e37_79b9_7f4a_7c15; // 2^64/φ
    let h2 = h1.rotate_left(17).wrapping_mul(PHI);

    (0..hash_count).map(move |i| h1.wrapping_add(h2.wrapping_mul(i as u64)))
}
