use crate::compression::stateless::bitops::{BitLength, BitsArray};
use crate::compression::stateless::constants::N_UNIQUE_BUCKETS;
use crate::compression::stateless::utils::felt_from_bits_le;
use indexmap::IndexMap;
use starknet_core::types::Felt;
use std::hash::Hash;

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
    fn try_from(bucket_element: BucketElement) -> color_eyre::Result<Self, Self::Error> {
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

// UniqueValueBucket
// Revert pack_in_felts signature
#[derive(Clone, Debug)]
struct UniqueValueBucket<SizedElement: BucketElementTrait + Eq + Hash> {
    value_to_index: IndexMap<SizedElement, usize>,
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
    pub(crate) fn bucket_indices(&self, bucket_element: &BucketElement) -> (usize, usize) {
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
    pub(crate) fn pack_in_felts(self) -> Vec<Felt> {
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
}
