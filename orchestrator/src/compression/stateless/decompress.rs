use crate::compression::stateless::bitops::BitLength;
use crate::compression::stateless::buckets::get_bucket_offsets;
use crate::compression::stateless::constants::{
    COMPRESSION_VERSION, HEADER_ELM_BOUND, HEADER_LEN, N_UNIQUE_BUCKETS, TOTAL_N_BUCKETS,
};
use crate::compression::stateless::utils::{felt_from_bits_le, felt_to_big_uint, get_n_elms_per_felt};
use color_eyre::eyre::{bail, eyre, Context};
use num_bigint::BigUint;
use num_traits::{ToPrimitive, Zero};
use starknet_core::types::Felt;
use std::cmp::{max, min};

// Decompression Logic
// Need unpack_chunk equivalent and match Python's reconstruction
// Helper function similar to Python's unpack_chunk
fn unpack_chunk(
    compressed_iter: &mut std::vec::IntoIter<Felt>,
    n_elms: usize,
    elm_bound: u32,
) -> color_eyre::Result<Vec<usize>> {
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
) -> color_eyre::Result<Vec<usize>> {
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
fn unpack_felt(packed_felt: Felt, elm_bound: u32, n_elms: usize) -> color_eyre::Result<Vec<usize>> {
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
pub fn decompress(compressed_data: &[Felt]) -> color_eyre::Result<Vec<Felt>> {
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
        .collect::<color_eyre::Result<_>>()?; // Collect results, propagating error

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
