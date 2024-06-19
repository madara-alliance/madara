//! Fact info structure and helpers.
//!
//! Port of https://github.com/starkware-libs/cairo-lang/blob/master/src/starkware/cairo/bootloaders/generate_fact.py

use alloy::primitives::{keccak256, B256};
use cairo_vm::program_hash::compute_program_hash_chain;
use cairo_vm::types::builtin_name::BuiltinName;
use cairo_vm::types::relocatable::MaybeRelocatable;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use cairo_vm::Felt252;
use starknet::core::types::FieldElement;
use utils::ensure;

use super::error::FactCheckerError;
use super::fact_node::generate_merkle_root;
use super::fact_topology::{get_fact_topology, FactTopology};

/// Default bootloader program version.
///
/// https://github.com/starkware-libs/cairo-lang/blob/efa9648f57568aad8f8a13fbf027d2de7c63c2c0/src/starkware/cairo/bootloaders/hash_program.py#L11
pub const BOOTLOADER_VERSION: usize = 0;

pub struct FactInfo {
    pub program_output: Vec<Felt252>,
    pub fact_topology: FactTopology,
    pub fact: B256,
}

pub fn get_fact_info(cairo_pie: &CairoPie, program_hash: Option<FieldElement>) -> Result<FactInfo, FactCheckerError> {
    let program_output = get_program_output(cairo_pie)?;
    let fact_topology = get_fact_topology(cairo_pie, program_output.len())?;
    let program_hash = match program_hash {
        Some(hash) => hash,
        None => compute_program_hash_chain(&cairo_pie.metadata.program, BOOTLOADER_VERSION)?,
    };
    let output_root = generate_merkle_root(&program_output, &fact_topology)?;
    let fact = keccak256([program_hash.to_bytes_be(), *output_root.node_hash].concat());
    Ok(FactInfo { program_output, fact_topology, fact })
}

pub fn get_program_output(cairo_pie: &CairoPie) -> Result<Vec<Felt252>, FactCheckerError> {
    let segment_info = cairo_pie
        .metadata
        .builtin_segments
        .get(&BuiltinName::output)
        .ok_or(FactCheckerError::OutputBuiltinNoSegmentInfo)?;

    let segment_start = cairo_pie
        .memory
        .0
        .iter()
        .enumerate()
        .find_map(|(ptr, ((index, _), _))| if *index == segment_info.index as usize { Some(ptr) } else { None })
        .ok_or(FactCheckerError::OutputSegmentNotFound)?;

    let mut output = Vec::with_capacity(segment_info.size);
    let mut expected_offset = 0;

    #[allow(clippy::explicit_counter_loop)]
    for i in segment_start..segment_start + segment_info.size {
        let ((_, offset), value) = cairo_pie.memory.0.get(i).ok_or(FactCheckerError::OutputSegmentInvalidRange)?;

        ensure!(
            *offset == expected_offset,
            FactCheckerError::OutputSegmentInconsistentOffset(*offset, expected_offset)
        );
        match value {
            MaybeRelocatable::Int(felt) => output.push(*felt),
            MaybeRelocatable::RelocatableValue(_) => {
                return Err(FactCheckerError::OutputSegmentUnexpectedRelocatable(*offset));
            }
        }

        expected_offset += 1;
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use cairo_vm::vm::runners::cairo_pie::CairoPie;

    use super::get_fact_info;

    #[test]
    fn test_fact_info() {
        // Generated using the get_fact.py script
        let expected_fact = "0xca15503f02f8406b599cb220879e842394f5cf2cef753f3ee430647b5981b782";
        let cairo_pie_path: PathBuf =
            [env!("CARGO_MANIFEST_DIR"), "tests", "artifacts", "fibonacci.zip"].iter().collect();
        let cairo_pie = CairoPie::read_zip_file(&cairo_pie_path).unwrap();
        let fact_info = get_fact_info(&cairo_pie, None).unwrap();
        assert_eq!(expected_fact, fact_info.fact.to_string());
    }
}
