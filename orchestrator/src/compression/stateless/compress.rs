use crate::compression::stateless::buckets::{get_bucket_offsets, BucketElement, Buckets};
use crate::compression::stateless::constants::{
    COMPRESSION_VERSION, HEADER_ELM_BOUND, N_UNIQUE_BUCKETS, TOTAL_N_BUCKETS,
};
use crate::compression::stateless::utils::{pack_usize_in_felt, pack_usize_in_felts};
use color_eyre::eyre::{eyre, Result};
use starknet_core::types::Felt;
use std::cmp::max;

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
    pub fn new(values: &[Felt]) -> Result<Self> {
        // Initialize Self with fields
        let mut obj = Self {
            unique_value_buckets: Buckets::new(),
            repeating_value_bucket: Vec::new(),
            bucket_index_per_elm: Vec::with_capacity(values.len()), // Use with_capacity
        };
        let repeating_values_bucket_index = N_UNIQUE_BUCKETS; // This is 6

        for value in values {
            // Use From trait (requires reverting BucketElement::From)
            let bucket_element = BucketElement::try_from(*value)?;
            let (bucket_index, inverse_bucket_index) = obj.unique_value_buckets.bucket_indices(&bucket_element);

            if let Some(element_index) = obj.unique_value_buckets.get_element_index(&bucket_element) {
                obj.repeating_value_bucket.push((bucket_index, *element_index));
                obj.bucket_index_per_elm.push(repeating_values_bucket_index);
            } else {
                obj.unique_value_buckets.add(bucket_element.clone());
                obj.bucket_index_per_elm.push(inverse_bucket_index);
            }
        }
        Ok(obj)
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
    pub fn pack_unique_values(self) -> Result<Vec<Felt>> {
        self.unique_value_buckets.pack_in_felts()
    }
}

// Compression Logic
// Revert compress signature and logic (no Result, use expect/panic)
pub fn compress(data: &[Felt]) -> Result<Vec<Felt>> {
    if data.len() >= HEADER_ELM_BOUND as usize {
        return Err(eyre!("Data is too long: {} >= {}", data.len(), HEADER_ELM_BOUND));
    }

    // Handle the empty case
    if data.is_empty() {
        let header: Vec<usize> = vec![COMPRESSION_VERSION.into(), 0, 0, 0, 0, 0, 0, 0, 0];
        // Return packed header directly, handle potential packing errors with expect/panic
        return Ok(vec![pack_usize_in_felt(&header, HEADER_ELM_BOUND)?]);
    }

    let compression_set = CompressionSet::new(data)?;

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
    let unique_values = compression_set.pack_unique_values()?; // Now returns Vec<Felt>

    Ok([vec![packed_header], unique_values, packed_repeating_value_pointers, packed_bucket_index_per_elm]
        .into_iter()
        .flatten()
        .collect())
}
