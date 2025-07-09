use std::any::type_name;
use std::cmp::max;
use std::convert::TryInto; // Needed for try_into() on slice
use std::hash::Hash;
use std::iter::Iterator; // Needed for decompress signature

use color_eyre::eyre::{bail, eyre, Context};
use color_eyre::Result;
use indexmap::IndexMap;
use itertools::Itertools;
use num_bigint::BigUint;
use num_traits::{ToPrimitive, Zero};
use starknet::core::types::Felt; // Assuming starknet re-exports Felt from starknet-types-core
use std::cmp::min;
use strum::EnumCount;
use strum_macros::Display;

// This code is mostly taken from Starkware's SNOS implementation. Here's the link for reference:
// https://github.com/starkware-libs/sequencer/blob/3b978f202e92714f07710c23d7d259ea6ca2f9e2/crates/starknet_os/src/hints/hint_implementation/stateless_compression/utils.rs

// Constants
pub(crate) const COMPRESSION_VERSION: u8 = 0;
pub(crate) const HEADER_ELM_N_BITS: usize = 20; // Max value ~1M
pub(crate) const HEADER_ELM_BOUND: u32 = 1 << HEADER_ELM_N_BITS;
pub(crate) const HEADER_LEN: usize = 1 + 1 + N_UNIQUE_BUCKETS + 1; // version, len, buckets, repeating_len
pub(crate) const N_UNIQUE_BUCKETS: usize = BitLength::COUNT;
pub(crate) const TOTAL_N_BUCKETS: usize = N_UNIQUE_BUCKETS + 1; // Includes repeating bucket
pub(crate) const MAX_N_BITS: usize = 251;

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
    const fn n_bits(&self) -> usize {
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
    pub(crate) fn min_bit_length(n_bits: usize) -> Result<Self> {
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

// BitsArray
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct BitsArray<const LENGTH: usize>(pub(crate) [bool; LENGTH]);

// Implementing TryFrom<Felt> for BitsArray<LENGTH>
impl<const LENGTH: usize> TryFrom<Felt> for BitsArray<LENGTH> {
    type Error = color_eyre::Report;

    fn try_from(felt: Felt) -> Result<Self, Self::Error> {
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
        let felt_as_biguint = felt_to_big_uint(&felt);
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

    fn try_from(bits_array: BitsArray<LENGTH>) -> Result<Self, Self::Error> {
        felt_from_bits_le(&bits_array.0)
    }
}

/// Converts a slice of bits into a Felt
/// Returns an error in case the length is not guaranteed to fit in Felt (more than 251 bits).
pub(crate) fn felt_from_bits_le(bits: &[bool]) -> Result<Felt> {
    if bits.len() > MAX_N_BITS {
        bail!("Value requires {} bits, exceeding limit for Felt", bits.len());
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
    // Check if Felt::from_bytes_le exists in the used starknet version
    // If not, use the from_bytes_be workaround
    // Assuming it doesn't exist based on previous findings:
    Ok(Felt::from_bytes_be(&bytes_le_to_be(&bytes)))
}

// Helper function to reverse the bytes of a byte slice
fn bytes_le_to_be(bytes_le: &[u8; 32]) -> [u8; 32] {
    let mut bytes_be = *bytes_le;
    bytes_be.reverse();
    bytes_be
}

// Helper function to convert Felt to BigUint
fn felt_to_big_uint(value: &Felt) -> BigUint {
    BigUint::from_bytes_be(&value.to_bytes_be())
}

// Creating BucketElements for different bit lengths
pub(crate) type BucketElement15 = BitsArray<15>;
pub(crate) type BucketElement31 = BitsArray<31>;
pub(crate) type BucketElement62 = BitsArray<62>;
pub(crate) type BucketElement83 = BitsArray<83>;
pub(crate) type BucketElement125 = BitsArray<125>;
pub(crate) type BucketElement252 = Felt;

// BucketElementTrait
// Modify trait to match an original structure (no bit_length, unpack_from_felts)
pub(crate) trait BucketElementTrait: Sized + Clone {
    fn pack_in_felts(elms: &[Self]) -> Vec<Felt>;
}

macro_rules! impl_bucket_element_trait {
    ($bucket_element:ident, $bit_length_enum:ident) => {
        // Removed $len parameter
        impl BucketElementTrait for $bucket_element {
            fn pack_in_felts(elms: &[Self]) -> Vec<Felt> {
                let bit_length = BitLength::$bit_length_enum;
                elms.chunks(bit_length.n_elems_in_felt())
                    .map(|chunk| {
                        felt_from_bits_le(&(chunk.iter().flat_map(|elem| elem.0.as_ref()).copied().collect::<Vec<_>>()))
                            .expect(&format!(
                                "Chunks of size {}, each of bit length {}, fit in felts.",
                                bit_length.n_elems_in_felt(),
                                bit_length
                            ))
                    })
                    .collect()
            }
        }
    };
}

impl_bucket_element_trait!(BucketElement15, Bits15);
impl_bucket_element_trait!(BucketElement31, Bits31);
impl_bucket_element_trait!(BucketElement62, Bits62);
impl_bucket_element_trait!(BucketElement83, Bits83);
impl_bucket_element_trait!(BucketElement125, Bits125);

impl BucketElementTrait for BucketElement252 {
    fn pack_in_felts(elms: &[Self]) -> Vec<Felt> {
        elms.to_vec()
    }
}

// BucketElement Enum
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum BucketElement {
    BucketElement15(BucketElement15),
    BucketElement31(BucketElement31),
    BucketElement62(BucketElement62),
    BucketElement83(BucketElement83),
    BucketElement125(BucketElement125),
    BucketElement252(BucketElement252),
}

// Revert From<Felt> to original logic (using expect)
// Note: This loses the nice Result propagation but matches the provided code
// If Result is preferred, keep the TryFrom implementation instead.
impl From<Felt> for BucketElement {
    fn from(felt: Felt) -> Self {
        match BitLength::min_bit_length(felt.bits()).expect("felt is up to 252 bits") {
            BitLength::Bits15 => BucketElement::BucketElement15(felt.try_into().expect("Up to 15 bits")),
            BitLength::Bits31 => BucketElement::BucketElement31(felt.try_into().expect("Up to 31 bits")),
            BitLength::Bits62 => BucketElement::BucketElement62(felt.try_into().expect("Up to 62 bits")),
            BitLength::Bits83 => BucketElement::BucketElement83(felt.try_into().expect("Up to 83 bits")),
            BitLength::Bits125 => BucketElement::BucketElement125(felt.try_into().expect("Up to 125 bits")),
            BitLength::Bits252 => BucketElement::BucketElement252(felt),
        }
    }
}

// Keep TryFrom<BucketElement> for Felt for decompression
impl TryFrom<BucketElement> for Felt {
    type Error = color_eyre::Report;
    fn try_from(bucket_element: BucketElement) -> Result<Self, Self::Error> {
        match bucket_element {
            BucketElement::BucketElement15(be) => Felt::try_from(be),
            BucketElement::BucketElement31(be) => Felt::try_from(be),
            BucketElement::BucketElement62(be) => Felt::try_from(be),
            BucketElement::BucketElement83(be) => Felt::try_from(be),
            BucketElement::BucketElement125(be) => Felt::try_from(be),
            BucketElement::BucketElement252(be) => Ok(be),
        }
    }
}

// UniqueValueBucket
// Revert pack_in_felts signature
#[derive(Clone, Debug)]
struct UniqueValueBucket<SizedElement: BucketElementTrait + Eq + Hash> {
    value_to_index: IndexMap<SizedElement, usize>,
}
impl<SizedElement: BucketElementTrait + Clone + Eq + Hash> UniqueValueBucket<SizedElement> {
    // ... new, len, contains, add, get_index ...
    fn new() -> Self {
        Self { value_to_index: Default::default() }
    }
    fn len(&self) -> usize {
        self.value_to_index.len()
    }
    fn contains(&self, value: &SizedElement) -> bool {
        self.value_to_index.contains_key(value)
    }
    fn add(&mut self, value: SizedElement) {
        if !self.contains(&value) {
            let next_index = self.value_to_index.len();
            self.value_to_index.insert(value, next_index);
        }
    }
    fn get_index(&self, value: &SizedElement) -> Option<&usize> {
        self.value_to_index.get(value)
    }

    fn pack_in_felts(self) -> Vec<Felt> {
        let values = self.value_to_index.into_keys().collect::<Vec<_>>();
        SizedElement::pack_in_felts(&values)
    }

    fn get_values(self) -> Vec<SizedElement> {
        self.value_to_index.into_keys().collect()
    }
}

// Buckets
#[derive(Clone, Debug)]
pub(crate) struct Buckets {
    // Add bucket fields
    bucket15: UniqueValueBucket<BucketElement15>,
    bucket31: UniqueValueBucket<BucketElement31>,
    bucket62: UniqueValueBucket<BucketElement62>,
    bucket83: UniqueValueBucket<BucketElement83>,
    bucket125: UniqueValueBucket<BucketElement125>,
    bucket252: UniqueValueBucket<BucketElement252>,
}
impl Buckets {
    // ... new, bucket_indices, get_element_index, add, lengths ...
    pub(crate) fn new() -> Self {
        Self {
            bucket15: UniqueValueBucket::new(),
            bucket31: UniqueValueBucket::new(),
            bucket62: UniqueValueBucket::new(),
            bucket83: UniqueValueBucket::new(),
            bucket125: UniqueValueBucket::new(),
            bucket252: UniqueValueBucket::new(),
        }
    }
    // Implement Buckets::bucket_indices
    // Returns (bucket_index, inverse_bucket_index)
    fn bucket_indices(&self, bucket_element: &BucketElement) -> (usize, usize) {
        let bucket_index = match bucket_element {
            BucketElement::BucketElement15(_) => 0,
            BucketElement::BucketElement31(_) => 1,
            BucketElement::BucketElement62(_) => 2,
            BucketElement::BucketElement83(_) => 3,
            BucketElement::BucketElement125(_) => 4,
            BucketElement::BucketElement252(_) => 5,
        };
        (bucket_index, N_UNIQUE_BUCKETS - 1 - bucket_index)
    }
    // Implement Buckets::get_element_index
    pub(crate) fn get_element_index(&self, bucket_element: &BucketElement) -> Option<&usize> {
        match bucket_element {
            BucketElement::BucketElement15(be) => self.bucket15.get_index(be),
            BucketElement::BucketElement31(be) => self.bucket31.get_index(be),
            BucketElement::BucketElement62(be) => self.bucket62.get_index(be),
            BucketElement::BucketElement83(be) => self.bucket83.get_index(be),
            BucketElement::BucketElement125(be) => self.bucket125.get_index(be),
            BucketElement::BucketElement252(be) => self.bucket252.get_index(be),
        }
    }
    // Implement Buckets::add
    pub(crate) fn add(&mut self, bucket_element: BucketElement) {
        match bucket_element {
            BucketElement::BucketElement15(be) => self.bucket15.add(be),
            BucketElement::BucketElement31(be) => self.bucket31.add(be),
            BucketElement::BucketElement62(be) => self.bucket62.add(be),
            BucketElement::BucketElement83(be) => self.bucket83.add(be),
            BucketElement::BucketElement125(be) => self.bucket125.add(be),
            BucketElement::BucketElement252(be) => self.bucket252.add(be),
        }
    }
    // Implement Buckets::lengths
    pub(crate) fn lengths(&self) -> [usize; N_UNIQUE_BUCKETS] {
        [
            self.bucket252.len(), // Order matters here for header
            self.bucket125.len(),
            self.bucket83.len(),
            self.bucket62.len(),
            self.bucket31.len(),
            self.bucket15.len(),
        ]
    }

    // Return Vec<Felt> not Result
    fn pack_in_felts(self) -> Vec<Felt> {
        [
            self.bucket15.pack_in_felts(),
            self.bucket31.pack_in_felts(),
            self.bucket62.pack_in_felts(),
            self.bucket83.pack_in_felts(),
            self.bucket125.pack_in_felts(),
            self.bucket252.pack_in_felts(),
        ]
        .into_iter()
        .rev()
        .flatten()
        .collect()
    }

    // Remove unpack_from_felts if not in an original concept

    // Gets all unique values ordered from the largest bit bucket to the smallest.
    // Keep this helper method as it's useful for decompress
    fn get_all_unique_values(self) -> Vec<BucketElement> {
        self.bucket252
            .get_values()
            .into_iter()
            .map(BucketElement::BucketElement252)
            .chain(self.bucket125.get_values().into_iter().map(BucketElement::BucketElement125))
            .chain(self.bucket83.get_values().into_iter().map(BucketElement::BucketElement83))
            .chain(self.bucket62.get_values().into_iter().map(BucketElement::BucketElement62))
            .chain(self.bucket31.get_values().into_iter().map(BucketElement::BucketElement31))
            .chain(self.bucket15.get_values().into_iter().map(BucketElement::BucketElement15))
            .collect()
    }
}

// CompressionSet
#[derive(Clone, Debug)]
pub(crate) struct CompressionSet {
    // Add fields
    unique_value_buckets: Buckets,
    repeating_value_bucket: Vec<(usize, usize)>, // (bucket_index, element_index)
    bucket_index_per_elm: Vec<usize>,
}
impl CompressionSet {
    // Revert new signature and logic to match the original (no Result, use expect)
    pub fn new(values: &[Felt]) -> Self {
        // Initialize Self with fields
        let mut obj = Self {
            unique_value_buckets: Buckets::new(),
            repeating_value_bucket: Vec::new(),
            bucket_index_per_elm: Vec::with_capacity(values.len()), // Use with_capacity
        };
        let repeating_values_bucket_index = N_UNIQUE_BUCKETS; // This is 6

        for value in values {
            // Use From trait (requires reverting BucketElement::From)
            let bucket_element = BucketElement::from(*value);
            let (bucket_index, inverse_bucket_index) = obj.unique_value_buckets.bucket_indices(&bucket_element);

            if let Some(element_index) = obj.unique_value_buckets.get_element_index(&bucket_element) {
                obj.repeating_value_bucket.push((bucket_index, *element_index));
                obj.bucket_index_per_elm.push(repeating_values_bucket_index);
            } else {
                obj.unique_value_buckets.add(bucket_element.clone());
                obj.bucket_index_per_elm.push(inverse_bucket_index);
            }
        }
        obj // Return Self directly
    }

    // ... get_unique_value_bucket_lengths, n_repeating_values ...
    pub fn get_unique_value_bucket_lengths(&self) -> [usize; N_UNIQUE_BUCKETS] {
        self.unique_value_buckets.lengths()
    }
    pub fn n_repeating_values(&self) -> usize {
        self.repeating_value_bucket.len()
    }

    // ... get_repeating_value_pointers ...
    pub fn get_repeating_value_pointers(&self) -> Vec<usize> {
        // Reconstruct repeating value pointers as expected by packing logic
        // The stored vec is (bucket_idx, element_idx), but we need just element_idx
        // Need to re-map element_index within a specific bucket to its global index across all unique values.

        // 1. Get unique lengths in the standard order (252, 125, ..., 15)
        let unique_lengths = self.unique_value_buckets.lengths();
        // 2. Calculate offsets based on these lengths
        let bucket_offsets = get_bucket_offsets(&unique_lengths); // Offsets for the global index (0=252..5=15)

        // 3. Map stored pointers (bucket_index=0..5 for 15b..252b, local_element_index) to global index
        self.repeating_value_bucket
            .iter()
            .map(|(bucket_index, index_in_bucket)| {
                // Need to map the stored bucket_index (0=15b..5=252b)
                // to the index used for bucket_offsets (0=252b..5=15b).
                // The mapping is: offset_index = N_UNIQUE_BUCKETS - 1 - bucket_index
                let offset_index = N_UNIQUE_BUCKETS - 1 - bucket_index;
                bucket_offsets[offset_index] + index_in_bucket
            })
            .collect()
    }

    // Return Vec<Felt> not Result
    pub fn pack_unique_values(self) -> Vec<Felt> {
        self.unique_value_buckets.pack_in_felts()
    }
}

// Compression Logic
// Revert compress signature and logic (no Result, use expect/panic)
pub fn compress(data: &[Felt]) -> Result<Vec<Felt>> {
    let data_len_usize = data.len();
    // Use assert! like Python version
    assert!(data_len_usize < HEADER_ELM_BOUND as usize, "Data is too long: {} >= {}", data_len_usize, HEADER_ELM_BOUND);

    // Handle the empty case
    if data.is_empty() {
        let header: Vec<usize> = vec![COMPRESSION_VERSION.into(), 0, 0, 0, 0, 0, 0, 0, 0];
        // Return packed header directly, handle potential packing errors with expect/panic
        return Ok(vec![pack_usize_in_felt(&header, HEADER_ELM_BOUND)?]);
    }

    let compression_set = CompressionSet::new(data); // Uses new which now returns Self

    let unique_value_bucket_lengths = compression_set.get_unique_value_bucket_lengths();
    let n_unique_values: usize = unique_value_bucket_lengths.iter().sum();

    // Use expect for conversions
    let n_unique_values_u32 =
        u32::try_from(n_unique_values).map_err(|err| eyre!("Too many unique values to fit in u32: {}", err))?;
    let repeating_pointers_bound = max(n_unique_values_u32, 1);

    let header: Vec<usize> = [COMPRESSION_VERSION.into(), data.len()]
        .into_iter()
        .chain(unique_value_bucket_lengths)
        .chain([compression_set.n_repeating_values()])
        .collect();

    // Use expect/panic where Results were previously handled
    let packed_header = pack_usize_in_felt(&header, HEADER_ELM_BOUND)?;
    let packed_repeating_value_pointers =
        pack_usize_in_felts(&compression_set.get_repeating_value_pointers(), repeating_pointers_bound)?;
    let packed_bucket_index_per_elm = pack_usize_in_felts(
        &compression_set.bucket_index_per_elm,
        u32::try_from(TOTAL_N_BUCKETS).map_err(|err| eyre!("TOTAL_N_BUCKETS does not fit in u32: {}", err))?,
    )?;
    let unique_values = compression_set.pack_unique_values(); // Now returns Vec<Felt>

    Ok([vec![packed_header], unique_values, packed_repeating_value_pointers, packed_bucket_index_per_elm]
        .into_iter()
        .flatten()
        .collect())
}

// Decompression Logic
// Need unpack_chunk equivalent and match Python's reconstruction
// Helper function similar to Python's unpack_chunk
fn unpack_chunk(compressed_iter: &mut std::vec::IntoIter<Felt>, n_elms: usize, elm_bound: u32) -> Result<Vec<usize>> {
    // Keep Result for unpacking errors
    if n_elms == 0 {
        return Ok(Vec::new());
    } // Handle zero elements case

    // Check elm_bound before calculating n_per_felt
    if elm_bound == 0 {
        bail!("Element bound cannot be 0 for unpacking chunk");
    }

    let n_elms_per_felt = get_n_elms_per_felt(elm_bound)?;
    if n_elms_per_felt == 0 {
        bail!("Calculated n_elms_per_felt is 0, likely due to too large elm_bound {}", elm_bound);
    }

    let n_packed_felts = n_elms.div_ceil(n_elms_per_felt);
    // let n_packed_felts = (n_elms + n_elms_per_felt - 1) / n_elms_per_felt;

    // Take exactly n_packed_felts from the iterator
    let compressed_chunk: Vec<Felt> = compressed_iter.take(n_packed_felts).collect();
    if compressed_chunk.len() != n_packed_felts {
        bail!("Insufficient felts in iterator: needed {}, got {}", n_packed_felts, compressed_chunk.len());
    }

    unpack_felts(compressed_chunk, elm_bound, n_elms)
}

// Need unpack_felts helper, equivalent to Python's
fn unpack_felts(
    compressed: Vec<Felt>, // Takes Vec now
    elm_bound: u32,
    n_elms: usize,
) -> Result<Vec<usize>> {
    // Keep Result for unpacking errors
    if elm_bound == 0 {
        bail!("Element bound cannot be 0 for unpacking felts");
    }
    let n_elms_per_felt = get_n_elms_per_felt(elm_bound)?;
    if n_elms_per_felt == 0 {
        bail!("Calculated n_elms_per_felt is 0 in unpack_felts, likely due to too large elm_bound {}", elm_bound);
    }

    let mut res = Vec::with_capacity(n_elms); // Estimate capacity

    for packed_felt in compressed {
        // Directly call unpack_felt helper (defined below)
        let unpacked = unpack_felt(packed_felt, elm_bound, n_elms_per_felt)?;
        res.extend(unpacked);
    }

    // Remove trailing zeros (Python does list(res)[:n_elms])
    res.truncate(n_elms);
    Ok(res)
}

// Need unpack_felt helper, equivalent to Python's
fn unpack_felt(packed_felt: Felt, elm_bound: u32, n_elms: usize) -> Result<Vec<usize>> {
    // Keep Result for unpacking errors
    if elm_bound == 0 {
        bail!("Element bound cannot be 0 for unpacking felt");
    }
    let mut res = Vec::with_capacity(n_elms);
    let mut current_felt_big = felt_to_big_uint(&packed_felt);
    let elm_bound_big = BigUint::from(elm_bound);
    for _ in 0..n_elms {
        // Use BigUint division and remainder
        let remainder_big = &current_felt_big % &elm_bound_big;
        let element =
            remainder_big.to_usize().ok_or_else(|| eyre!("usize conversion failed for value: {}", remainder_big))?;
        res.push(element);
        current_felt_big /= &elm_bound_big; // Integer division
    }

    if !current_felt_big.is_zero() {
        // Python asserts packed_felt == 0 here. Let's make it an error.
        bail!("Non-zero remainder after unpacking felt: {}", current_felt_big);
    }
    Ok(res)
}

// Rewrite decompress using unpack_chunk and Python's reconstruction logic
pub fn decompress(compressed_data: &[Felt]) -> Result<Vec<Felt>> {
    // Keep Result for error handling
    if compressed_data.is_empty() {
        return Ok(Vec::new());
    }
    // Special check for the single packed header of an empty list
    if compressed_data.len() == 1 {
        let packed_header_felt = compressed_data[0];
        // Try to unpack the single felt header
        let header =
            unpack_felt(packed_header_felt, HEADER_ELM_BOUND, HEADER_LEN).context("Failed to unpack header felt")?;
        // Check if it's the header for an empty list (version=0, data_len=0, rest=0)
        if header.len() == HEADER_LEN
            && header[0] == COMPRESSION_VERSION as usize
            && header[1] == 0
            && header[2..].iter().all(|&x| x == 0)
        {
            return Ok(Vec::new());
        }
        bail!("Invalid compressed data: single non-empty header felt provided.");
    }

    let mut felt_iter = compressed_data.to_vec().into_iter(); // Consumable iterator

    // 1. Unpack Header (single felt)
    let packed_header_felt = felt_iter.next().ok_or_else(|| eyre!("Compressed data is too short, missing header."))?;
    let header =
        unpack_felt(packed_header_felt, HEADER_ELM_BOUND, HEADER_LEN).context("Failed to unpack header felt")?;

    let version = header[0];
    if version != COMPRESSION_VERSION as usize {
        bail!("Unsupported compression version: {}", version);
    }
    let data_len = header[1];
    if data_len == 0 {
        return Ok(Vec::new());
    } // Handle case where data len was 0

    let unique_bucket_lengths: Vec<usize> = header[2..2 + N_UNIQUE_BUCKETS].to_vec(); // As Vec
    let n_repeating_values = header[2 + N_UNIQUE_BUCKETS];

    // 2. Unpack Unique Values
    let mut unique_values = Vec::new();
    // Unpack 252-bit bucket (raw Felts)
    unique_values.extend(felt_iter.by_ref().take(unique_bucket_lengths[0]));

    // Unpack other buckets using bit-level reconstruction
    let bit_lengths_enum =
        [BitLength::Bits125, BitLength::Bits83, BitLength::Bits62, BitLength::Bits31, BitLength::Bits15];
    for (i, bit_length) in bit_lengths_enum.iter().enumerate() {
        let bucket_len = unique_bucket_lengths[i + 1]; // Offset by 1 because 252 was index 0
        if bucket_len > 0 {
            let n_bits = bit_length.n_bits();
            let n_elms_per_felt = bit_length.n_elems_in_felt();
            // Use div_ceil equivalent: (a + b - 1) / b
            // let n_packed_felts = (bucket_len + n_elms_per_felt - 1) / n_elms_per_felt;
            let n_packed_felts = bucket_len.div_ceil(n_elms_per_felt);

            let packed_felts: Vec<Felt> = felt_iter.by_ref().take(n_packed_felts).collect();
            if packed_felts.len() != n_packed_felts {
                bail!(
                    "Insufficient felts for {}-bit bucket (needed {}, got {})",
                    n_bits,
                    n_packed_felts,
                    packed_felts.len()
                );
            }

            let mut current_unpacked_count = 0;
            for packed_felt in packed_felts {
                let n_to_unpack_from_this_felt = min(n_elms_per_felt, bucket_len - current_unpacked_count);
                let mut current_bits = Vec::new();
                let felt_as_biguint = felt_to_big_uint(&packed_felt);

                // Extract all the necessary bits from the felt
                // Note: This assumes LE bit order packing, matching felt_from_bits_le
                let total_bits_needed = n_to_unpack_from_this_felt * n_bits;
                for bit_idx in 0..total_bits_needed {
                    current_bits.push(felt_as_biguint.bit(bit_idx as u64));
                }

                // Reconstruct values from chunks of bits
                for bit_chunk in current_bits.chunks_exact(n_bits) {
                    let value = felt_from_bits_le(bit_chunk)
                        .with_context(|| format!("Failed to reconstruct Felt from {}-bit chunk", n_bits))?;
                    unique_values.push(value);
                    current_unpacked_count += 1;
                    if current_unpacked_count == bucket_len {
                        break;
                    } // Stop if we unpacked all needed
                }
                if current_unpacked_count == bucket_len {
                    break;
                } // Stop outer loop too
            }
            if current_unpacked_count != bucket_len {
                bail!(
                    "Failed to unpack expected number of elements for {}-bit bucket (expected {}, got {})",
                    n_bits,
                    bucket_len,
                    current_unpacked_count
                );
            }
        }
    }

    let n_unique_values = unique_values.len();
    let unique_values_bound = max(n_unique_values as u32, 1);

    // 3. Unpack Repeating Value Pointers
    let repeating_value_pointers = unpack_chunk(&mut felt_iter, n_repeating_values, unique_values_bound)
        .context("Failed to unpack repeating value pointers")?;

    // 4. Create `all_values` list (unique + repeating)
    let repeating_values: Vec<Felt> = repeating_value_pointers
        .iter()
        .map(|&ptr| {
            unique_values
                .get(ptr)
                .cloned()
                .ok_or_else(|| eyre!("Repeating pointer index {} out of bounds {}", ptr, n_unique_values))
        })
        .collect::<Result<_>>()?; // Collect results, propagating error

    let mut all_values = unique_values; // Start with unique
    all_values.extend(repeating_values); // Add repeating

    // 5. Unpack Bucket Index Per Element
    let bucket_index_per_elm = unpack_chunk(&mut felt_iter, data_len, TOTAL_N_BUCKETS as u32) // Use TOTAL_N_BUCKETS as bound
        .context("Failed to unpack bucket indices")?;

    // Check consumption
    let remaining_felts: Vec<Felt> = felt_iter.collect();
    if !remaining_felts.is_empty() && !remaining_felts.iter().all(|f| *f == Felt::ZERO) {
        eprintln!(
            "Warning: Extra non-zero data found after unpacking ({} felts): {:?}",
            remaining_felts.len(),
            remaining_felts
        );
    }

    // 6. Reconstruct using Python logic
    let all_bucket_lengths =
        unique_bucket_lengths.iter().copied().chain(std::iter::once(n_repeating_values)).collect::<Vec<_>>();
    let all_bucket_offsets = get_bucket_offsets(&all_bucket_lengths); // Use helper

    // Create iterators (Rust equivalent of count(start=offset))
    let mut bucket_offset_iterators: Vec<_> = all_bucket_offsets.into_iter().map(|offset| offset..).collect(); // Infinite range iterators

    let mut original_data = Vec::with_capacity(data_len);
    for bucket_index in bucket_index_per_elm {
        if bucket_index >= bucket_offset_iterators.len() {
            bail!("Bucket index {} out of bounds for offset iterators", bucket_index);
        }
        // Get the next global index from the correct iterator
        let global_index = bucket_offset_iterators[bucket_index]
            .next()
            .ok_or_else(|| eyre!("Offset iterator {} exhausted unexpectedly", bucket_index))?;

        // Get value from all_values
        let value = all_values.get(global_index).ok_or_else(|| {
            eyre!("Global index {} out of bounds for all_values (len {})", global_index, all_values.len())
        })?;
        original_data.push(*value);
    }

    if original_data.len() != data_len {
        bail!("Final length mismatch: expected {}, got {}", data_len, original_data.len());
    }

    Ok(original_data)
}

// Packing/Unpacking Utilities
// Revert signatures (no Result) and use expect/panic
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

pub fn pack_usize_in_felts(elms: &[usize], elm_bound: u32) -> Result<Vec<Felt>> {
    if elm_bound == 0 {
        return Err(eyre!("Element bound cannot be 0 for packing"));
    }
    // Check elements are within bound
    for elm in elms {
        let elm_u32 = u32::try_from(*elm).map_err(|err| eyre!("Cannot convert element to u32: {}", err))?;
        assert!(elm_u32 < elm_bound, "Element {} exceeds bound {}", elm, elm_bound);
    }

    let n_per_felt = get_n_elms_per_felt(elm_bound)?;
    if n_per_felt == 0 {
        return Err(eyre!("Element bound {} too large to fit in Felt", elm_bound));
    }

    Ok(elms.chunks(n_per_felt).map(|chunk| pack_usize_in_felt(chunk, elm_bound)).try_collect()?)
}

fn pack_usize_in_felt(elms: &[usize], elm_bound: u32) -> Result<Felt> {
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

// get_bucket_offsets needs slice input like original
pub(crate) fn get_bucket_offsets(bucket_lengths: &[usize]) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(bucket_lengths.len());
    let mut current = 0;

    for &length in bucket_lengths {
        offsets.push(current);
        current += length;
    }
    offsets
}
