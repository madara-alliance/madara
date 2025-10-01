//! Fact info structure and helpers.
//!
//! Port of https://github.com/starkware-libs/cairo-lang/blob/master/src/starkware/cairo/bootloaders/generate_fact.py

use super::fact_node::generate_merkle_root;
use super::fact_topology::{get_fact_topology, FactTopology};
use crate::error::job::fact::FactError;
use crate::types::constant::BOOT_LOADER_PROGRAM_CONTRACT;
use alloy::primitives::{keccak256, B256};
use cairo_vm::program_hash::compute_program_hash_chain;
use cairo_vm::types::builtin_name::BuiltinName;
use cairo_vm::types::relocatable::MaybeRelocatable;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use cairo_vm::Felt252;
use num_traits::ToPrimitive;
use orchestrator_ethereum_settlement_client::N_BLOBS_OFFSET;
use serde::{Deserialize, Serialize};
use starknet::core::types::Felt;
use starknet_crypto::poseidon_hash_many;
use std::ops::Add;
// use std::str::FromStr; // Unused

/// Default bootloader program version.
///
/// https://github.com/starkware-libs/cairo-lang/blob/efa9648f57568aad8f8a13fbf027d2de7c63c2c0/src/starkware/cairo/bootloaders/hash_program.py#L11
pub const BOOTLOADER_VERSION: usize = 0;

pub struct FactInfo {
    pub program_output: Vec<Felt252>,
    pub fact_topology: FactTopology,
    pub fact: B256,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OnChainData {
    pub on_chain_data_hash: B256,
    pub on_chain_data_size: usize,
}

pub struct BootLoaderOutput {
    pub one_felt: Felt,
    pub program_output_len_add_2: Felt,
    pub program_hash: Felt,
    pub program_output: Vec<Felt>,
}

impl BootLoaderOutput {
    pub fn to_byte_nested_vec(&self) -> Vec<Vec<u8>> {
        let mut result = Vec::new();
        result.push(self.one_felt.to_bytes_be().to_vec());
        result.push(self.program_output_len_add_2.to_bytes_be().to_vec());
        result.push(self.program_hash.to_bytes_be().to_vec());
        for felt in &self.program_output {
            result.push(felt.to_bytes_be().to_vec());
        }
        result
    }
}

pub fn get_fact_l2(cairo_pie: &CairoPie, program_hash: Option<Felt>) -> color_eyre::Result<B256> {
    let program_hash = match program_hash {
        Some(hash) => hash,
        None => Felt::from_bytes_be(
            &compute_program_hash_chain(&cairo_pie.metadata.program, BOOTLOADER_VERSION)
                .map_err(|e| {
                    tracing::error!(
                        log_type = "FactInfo",
                        category = "fact_info",
                        function_type = "get_fact_info",
                        "Failed to compute program hash: {}",
                        e
                    );
                    FactError::ProgramHashCompute(e.to_string())
                })?
                .to_bytes_be(),
        ),
    };

    let program_output = get_program_output(cairo_pie, false)?;
    let boot_loader_output: BootLoaderOutput = BootLoaderOutput {
        one_felt: Felt::ONE,
        program_output_len_add_2: Felt::from(program_output.len()).add(2),
        program_hash,
        program_output,
    };
    // Convert boot loader output to Felt values for poseidon hash
    let boot_loader_output_vec = boot_loader_output.program_output.to_vec(); // Get the program output Vec<Felt>
    let boot_loader_output_hash = poseidon_hash_many(&boot_loader_output_vec);

    let boot_loader_program_hash = Felt::from_hex(BOOT_LOADER_PROGRAM_CONTRACT)
        .map_err(|e| color_eyre::eyre::eyre!("Failed to parse boot loader program hash: {}", e))?;

    let fact_hash = poseidon_hash_many(&[boot_loader_program_hash, boot_loader_output_hash]);

    Ok(B256::from_slice(&fact_hash.to_bytes_be()))
}

pub fn build_on_chain_data(cairo_pie: &CairoPie) -> color_eyre::Result<OnChainData> {
    let program_output = get_program_output(cairo_pie, false)?;
    let fact_topology = get_fact_topology(cairo_pie, program_output.len())?;
    let fact_root = generate_merkle_root(&program_output, &fact_topology)?;

    let da_child = fact_root.children.last().expect("fact_root is empty");

    Ok(OnChainData { on_chain_data_hash: da_child.node_hash, on_chain_data_size: da_child.page_size })
}

pub fn get_fact_info(
    cairo_pie: &CairoPie,
    program_hash: Option<Felt>,
    is_aggregator: bool,
) -> Result<FactInfo, FactError> {
    tracing::debug!(
        log_type = "FactInfo",
        category = "fact_info",
        function_type = "get_fact_info",
        "Starting get_fact_info function"
    );

    tracing::debug!(
        log_type = "FactInfo",
        category = "fact_info",
        function_type = "get_fact_info",
        "Getting program output"
    );
    let program_output = get_program_output(cairo_pie, is_aggregator)?;
    tracing::debug!(
        log_type = "FactInfo",
        category = "fact_info",
        function_type = "get_fact_info",
        "Program output length: {}",
        program_output.len()
    );

    tracing::debug!(
        log_type = "FactInfo",
        category = "fact_info",
        function_type = "get_fact_info",
        "Getting fact topology"
    );
    let fact_topology = get_fact_topology(cairo_pie, program_output.len())?;

    let program_hash = match program_hash {
        Some(hash) => {
            tracing::debug!(
                log_type = "FactInfo",
                category = "fact_info",
                function_type = "get_fact_info",
                "Using provided program hash"
            );
            hash
        }
        None => {
            tracing::debug!(
                log_type = "FactInfo",
                category = "fact_info",
                function_type = "get_fact_info",
                "Computing program hash"
            );
            Felt::from_bytes_be(
                &compute_program_hash_chain(&cairo_pie.metadata.program, BOOTLOADER_VERSION)
                    .map_err(|e| {
                        tracing::error!(
                            log_type = "FactInfo",
                            category = "fact_info",
                            function_type = "get_fact_info",
                            "Failed to compute program hash: {}",
                            e
                        );
                        FactError::ProgramHashCompute(e.to_string())
                    })?
                    .to_bytes_be(),
            )
        }
    };
    tracing::trace!(
        log_type = "FactInfo",
        category = "fact_info",
        function_type = "get_fact_info",
        "Program hash: {:?} and now generating merkle root",
        program_hash
    );
    let output_root = generate_merkle_root(&program_output, &fact_topology)?;
    let fact = keccak256([program_hash.to_bytes_be(), *output_root.node_hash].concat());
    tracing::debug!(
        log_type = "FactInfo",
        category = "fact_info",
        function_type = "get_fact_info",
        "Fact computed successfully: {:?}",
        fact
    );

    Ok(FactInfo { program_output, fact_topology, fact })
}

/// The output of the aggregator program returns both the input and the output to it.
/// This is because it is used further by applicative bootloader which requires it.
/// This function is used to filter the output from the complete output
/// The program output contains an array of Felts. We filter out the output part from this file.
/// The data is in the following format (Let P = Program output array)
///
/// 0 = Number of blocks. Let it be `m`. \
/// 1 = The input size of the first block. Let it be N1. \
/// The next N1 indexes contain input data for block 1. \
/// 1 + N1 + 1 = The input size of the second block. Let it be N2. \
/// The next N2 indexes contain input data for block 2. \
/// And so on till `m` blocks. \
/// After that starts the output which we extract.
///
/// Since there can be 0s appended at the end, we carefully extract the information in the output
/// and don't touch the trailing 0s.
///
/// Check any transaction on the Starknet core contract to see the program output format.
/// For eg. https://sepolia.etherscan.io/tx/0x30c2280d308948aa727b9345752cc71c099a09612227ccc908a216fd06195001
pub fn filter_output_from_program_output(program_output: Vec<Felt252>) -> Result<Vec<Felt252>, FactError> {
    // Length needs to be at least 1 so that we can get the number of blocks
    if program_output.is_empty() {
        return Err(FactError::AggregatorOutputParsingError);
    }

    let num_blocks = program_output[0].to_usize().ok_or(FactError::FeltToUsizeConversionError)?;

    let mut output_start = 1;
    for _ in 0..num_blocks {
        let block_size = program_output
            .get(output_start)
            .ok_or(FactError::AggregatorOutputParsingError)?
            .to_usize()
            .ok_or(FactError::FeltToUsizeConversionError)?;
        output_start += block_size;
    }

    let n_blobs = program_output
        .get(output_start + N_BLOBS_OFFSET)
        .ok_or(FactError::AggregatorOutputParsingError)?
        .to_usize()
        .ok_or(FactError::FeltToUsizeConversionError)?;
    let message_start = output_start + N_BLOBS_OFFSET + n_blobs * 2 * 2 + 1;
    let n_l2_to_l1_messages = program_output
        .get(message_start)
        .ok_or(FactError::AggregatorOutputParsingError)?
        .to_usize()
        .ok_or(FactError::FeltToUsizeConversionError)?;
    let n_l1_to_l2_messages = program_output
        .get(message_start + n_l2_to_l1_messages + 1)
        .ok_or(FactError::AggregatorOutputParsingError)?
        .to_usize()
        .ok_or(FactError::FeltToUsizeConversionError)?;
    let message_end = message_start + n_l2_to_l1_messages + 1 + n_l1_to_l2_messages;

    if message_end >= program_output.len() {
        return Err(FactError::AggregatorOutputParsingError);
    }

    Ok(program_output[output_start..(message_end + 1)].to_vec())
}

pub fn get_program_output(cairo_pie: &CairoPie, is_aggregator: bool) -> Result<Vec<Felt252>, FactError> {
    let segment_info =
        cairo_pie.metadata.builtin_segments.get(&BuiltinName::output).ok_or(FactError::OutputBuiltinNoSegmentInfo)?;

    let mut output = vec![Felt252::from(0); segment_info.size];
    let mut insertion_count = 0;
    let cairo_program_memory = &cairo_pie.memory.0;

    for ((index, offset), value) in cairo_program_memory.iter() {
        if *index == segment_info.index as usize {
            match value {
                MaybeRelocatable::Int(felt) => {
                    output[*offset] = *felt;
                    insertion_count += 1;
                }
                MaybeRelocatable::RelocatableValue(_) => {
                    return Err(FactError::OutputSegmentUnexpectedRelocatable(*offset));
                }
            }
        }
    }

    if insertion_count != segment_info.size {
        return Err(FactError::InvalidSegment);
    }

    Ok(if is_aggregator { filter_output_from_program_output(output)? } else { output })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use cairo_vm::vm::runners::cairo_pie::CairoPie;
    use rstest::rstest;

    use super::get_fact_info;

    #[rstest]
    #[case("fibonacci.zip", "0xca15503f02f8406b599cb220879e842394f5cf2cef753f3ee430647b5981b782")]
    #[case("sepolia_924016.zip", "0x033144ad6b132b1012f15e50aa53de1ed91b3b1af729014c3f1c00b702f972ea")]
    async fn test_fact_info(#[case] cairo_pie_path: &str, #[case] expected_fact: &str) {
        // TODO: Add a test for the aggregator program
        dotenvy::from_filename_override("../.env.test").expect("Failed to load the .env.test file");
        let cairo_pie_path: PathBuf =
            [env!("CARGO_MANIFEST_DIR"), "src", "tests", "artifacts", cairo_pie_path].iter().collect();
        let cairo_pie = CairoPie::read_zip_file(&cairo_pie_path).unwrap();
        let fact_info = get_fact_info(&cairo_pie, None, false).unwrap();
        assert_eq!(expected_fact, fact_info.fact.to_string());
    }
}
