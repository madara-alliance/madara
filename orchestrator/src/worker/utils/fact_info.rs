//! Fact info structure and helpers.
//!
//! Port of https://github.com/starkware-libs/cairo-lang/blob/master/src/starkware/cairo/bootloaders/generate_fact.py

use super::fact_node::generate_merkle_root;
use super::fact_topology::{get_fact_topology, FactTopology};
use crate::error::job::fact::FactError;
use alloy::primitives::{keccak256, B256};
use cairo_vm::program_hash::compute_program_hash_chain;
use cairo_vm::types::builtin_name::BuiltinName;
use cairo_vm::types::relocatable::MaybeRelocatable;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use cairo_vm::Felt252;
use serde::{Deserialize, Serialize};
use starknet::core::types::Felt;
use starknet_os::crypto::poseidon::poseidon_hash_many_bytes;
use std::ops::Add;
use std::str::FromStr;

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

    let program_output = get_program_output(cairo_pie)?;
    let boot_loader_output: BootLoaderOutput = BootLoaderOutput {
        one_felt: Felt::ONE,
        program_output_len_add_2: Felt::from(program_output.len()).add(2),
        program_hash,
        program_output,
    };
    let boot_loader_output_slice_vec = boot_loader_output.to_byte_nested_vec();
    let boot_loader_output_hash_vec =
        poseidon_hash_many_bytes(&boot_loader_output_slice_vec.iter().map(|v| v.as_slice()).collect::<Vec<_>>())?
            .to_vec();
    let boot_loader_output_hash = boot_loader_output_hash_vec.as_slice();

    // Bootloader Program Hash : 0x5ab580b04e3532b6b18f81cfa654a05e29dd8e2352d88df1e765a84072db07
    // taken from the code sent by integrity team.
    let boot_loader_program_hash_bytes =
        Felt::from_str("0x5ab580b04e3532b6b18f81cfa654a05e29dd8e2352d88df1e765a84072db07")?.to_bytes_be();
    let boot_loader_program_hash = boot_loader_program_hash_bytes.as_slice();

    let fact_hash = poseidon_hash_many_bytes(vec![boot_loader_program_hash, boot_loader_output_hash].as_slice())?;

    Ok(B256::from_slice(fact_hash.to_vec().as_slice()))
}

pub fn build_on_chain_data(cairo_pie: &CairoPie) -> color_eyre::Result<OnChainData> {
    let program_output = get_program_output(cairo_pie)?;
    let fact_topology = get_fact_topology(cairo_pie, program_output.len())?;
    let fact_root = generate_merkle_root(&program_output, &fact_topology)?;

    let da_child = fact_root.children.last().expect("fact_root is empty");

    Ok(OnChainData { on_chain_data_hash: da_child.node_hash, on_chain_data_size: da_child.page_size })
}

pub fn get_fact_info(cairo_pie: &CairoPie, program_hash: Option<Felt>) -> Result<FactInfo, FactError> {
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
    let program_output = get_program_output(cairo_pie)?;
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

pub fn get_program_output(cairo_pie: &CairoPie) -> Result<Vec<Felt252>, FactError> {
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

    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use cairo_vm::vm::runners::cairo_pie::CairoPie;
    use rstest::rstest;

    use super::get_fact_info;

    #[rstest]
    #[case("fibonacci.zip", "0xca15503f02f8406b599cb220879e842394f5cf2cef753f3ee430647b5981b782")]
    #[case("238996-SN.zip", "0xec8fa9cdfe069ed59b8f17aeecfd95c6abd616379269d2fa16a80955b6e0f068")]
    async fn test_fact_info(#[case] cairo_pie_path: &str, #[case] expected_fact: &str) {
        dotenvy::from_filename_override("../.env.test").expect("Failed to load the .env.test file");
        let cairo_pie_path: PathBuf =
            [env!("CARGO_MANIFEST_DIR"), "src", "tests", "artifacts", cairo_pie_path].iter().collect();
        let cairo_pie = CairoPie::read_zip_file(&cairo_pie_path).unwrap();
        let fact_info = get_fact_info(&cairo_pie, None).unwrap();
        assert_eq!(expected_fact, fact_info.fact.to_string());
    }
}
