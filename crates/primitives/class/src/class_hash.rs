use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use serde_json::ser::{Formatter, escape_str};
use sha3::{Digest, Keccak256};
use starknet_core::{
    types::contract::legacy::{
        LegacyContractClass, LegacyEntrypointOffset, LegacyProgram, RawLegacyEntryPoint,
        RawLegacyEntryPoints,
    },
    utils::starknet_keccak,
};
use starknet_types_core::{
    felt::Felt,
    hash::{Poseidon, StarkHash},
};

use crate::{
    convert::parse_compressed_legacy_class, CompressedLegacyContractClass, ContractClass, FlattenedSierraClass, SierraEntryPoint
};

use std::io::{self, Read};

#[derive(Debug, thiserror::Error)]
pub enum ComputeClassHashError {
    #[error("Unsupported Sierra version: {0}")]
    UnsupportedSierraVersion(String),
    #[error("Serialization error: {0}")]
    SerializationError(String),
}

impl ContractClass {
    pub fn compute_class_hash(&self) -> Result<Felt, ComputeClassHashError> {
        match self {
            ContractClass::Sierra(sierra) => sierra.compute_class_hash(),
            ContractClass::Legacy(legacy) => legacy.compute_class_hash(),
        }
    }
}

const SIERRA_VERSION: Felt =
    Felt::from_hex_unchecked("0x434f4e54524143545f434c4153535f56302e312e30"); // b"CONTRACT_CLASS_V0.1.0"

impl FlattenedSierraClass {
    pub fn compute_class_hash(&self) -> Result<Felt, ComputeClassHashError> {
        if self.contract_class_version != "0.1.0" {
            return Err(ComputeClassHashError::UnsupportedSierraVersion(
                self.contract_class_version.clone(),
            ));
        }

        let external_hash = compute_hash_entries_point(&self.entry_points_by_type.external);
        let l1_handler_hash = compute_hash_entries_point(&self.entry_points_by_type.l1_handler);
        let constructor_hash = compute_hash_entries_point(&self.entry_points_by_type.constructor);
        let abi_hash = starknet_keccak(self.abi.as_bytes());
        let program_hash = Poseidon::hash_array(&self.sierra_program);

        Ok(Poseidon::hash_array(&[
            SIERRA_VERSION,
            external_hash,
            l1_handler_hash,
            constructor_hash,
            Felt::from_bytes_be(&abi_hash),
            program_hash,
        ]))
    }
}

fn compute_hash_entries_point(entry_points: &[SierraEntryPoint]) -> Felt {
    let entry_point_flatten: Vec<_> = entry_points
        .iter()
        .flat_map(|SierraEntryPoint { selector, function_idx }| {
            [*selector, Felt::from(*function_idx)].into_iter()
        })
        .collect();
    Poseidon::hash_array(&entry_point_flatten)
}

impl CompressedLegacyContractClass {
    pub fn compute_class_hash(&self) -> Result<Felt, ComputeClassHashError> {
        let legacy_contract_class = parse_compressed_legacy_class(self.clone())?;

        // Step 1: Modify the contract class definition
        let mut modified_program = legacy_contract_class.program.clone();
        modified_program.debug_info = None;

        // Remove empty or null `accessible_scopes` and `flow_tracking_data`
        remove_empty_attributes(&mut modified_program.attributes)?;

        // Handle named tuples in `identifiers` and `reference_manager`
        if modified_program.compiler_version.is_none() {
            add_extra_space_to_cairo_named_tuples(&mut modified_program.identifiers);
            add_extra_space_to_cairo_named_tuples(&mut modified_program.reference_manager);
        }

        // Step 2: Serialize the modified definition
        let serialized_json = serialize_program(&modified_program)?;

        // Step 3: Compute the Keccak256 hash and truncate it
        let truncated_keccak = truncated_keccak(&serialized_json);

        // Step 4: Build the hash chain
        let class_hash = compute_hash_chain(
            &legacy_contract_class.entry_points_by_type,
            &modified_program.builtins,
            &modified_program.data,
            truncated_keccak,
        )?;

        Ok(class_hash)
    }
}

fn remove_empty_attributes(
    attributes: &mut Vec<serde_json::Value>,
) -> Result<(), ComputeClassHashError> {
    for attr in attributes.iter_mut() {
        let attr_obj = attr.as_object_mut().ok_or_else(|| {
            ComputeClassHashError::SerializationError("Attribute is not an object".to_string())
        })?;

        if let Some(serde_json::Value::Array(array)) = attr_obj.get("accessible_scopes") {
            if array.is_empty() {
                attr_obj.remove("accessible_scopes");
            }
        }

        if let Some(serde_json::Value::Null) = attr_obj.get("flow_tracking_data") {
            attr_obj.remove("flow_tracking_data");
        }
    }
    Ok(())
}

fn add_extra_space_to_cairo_named_tuples(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Array(v) => {
            for val in v {
                add_extra_space_to_cairo_named_tuples(val);
            }
        }
        serde_json::Value::Object(m) => {
            for (k, v) in m.iter_mut() {
                match v {
                    serde_json::Value::String(s) => {
                        if k == "cairo_type" || k == "value" {
                            *v = serde_json::Value::String(add_extra_space_before_colon(s));
                        }
                    }
                    _ => add_extra_space_to_cairo_named_tuples(v),
                }
            }
        }
        _ => {}
    }
}

fn add_extra_space_before_colon(s: &str) -> String {
    s.replace(": ", " : ").replace("  :", " :")
}

fn serialize_program(program: &LegacyProgram) -> Result<String, ComputeClassHashError> {
    // Use a custom formatter to match Python's JSON serialization
    let mut buffer = Vec::new();
    let mut serializer =
        serde_json::Serializer::with_formatter(&mut buffer, PythonDefaultFormatter);
    program
        .serialize(&mut serializer)
        .map_err(|e| ComputeClassHashError::SerializationError(format!("Serialize error: {}", e)))?;

    let serialized_json = String::from_utf8(buffer).map_err(|e| {
        ComputeClassHashError::SerializationError(format!("UTF-8 error: {}", e))
    })?;

    Ok(serialized_json)
}

fn truncated_keccak(serialized_json: &str) -> Felt {
    let mut hasher = Keccak256::new();
    hasher.update(serialized_json.as_bytes());
    let mut result = hasher.finalize();
    result[0] &= 0x03; // Truncate to 250 bits by masking the first 6 bits
    Felt::from_bytes_be(&result[..])
}

fn compute_hash_chain(
    entry_points_by_type: &RawLegacyEntryPoints,
    builtins: &[String],
    data: &[String],
    truncated_keccak: Felt,
) -> Result<Felt, ComputeClassHashError> {
    let api_version = Felt::ZERO;

    let mut outer = HashChain::default();
    outer.update(api_version);

    // Entry points
    let entry_point_types = [
        &entry_points_by_type.external,
        &entry_points_by_type.l1_handler,
        &entry_points_by_type.constructor,
    ];
    for entry_points in entry_point_types.iter() {
        let mut hash_chain = HashChain::default();
        for entry_point in entry_points.iter() {
            hash_chain.update(entry_point.selector);
            let offset_felt = match &entry_point.offset {
                LegacyEntrypointOffset::U64AsHex(offset) => Felt::from(*offset),
                LegacyEntrypointOffset::U64AsInt(offset) => Felt::from(*offset),
            };
            hash_chain.update(offset_felt);
        }
        outer.update(hash_chain.finalize());
    }

    // Builtins
    let mut builtins_hash_chain = HashChain::default();
    for builtin in builtins {
        let keccak_hash = starknet_keccak(builtin.as_bytes());
        let builtin_felt = Felt::from_bytes_be(&keccak_hash);
        builtins_hash_chain.update(builtin_felt);
    }
    outer.update(builtins_hash_chain.finalize());

    // Truncated Keccak
    outer.update(truncated_keccak);

    // Bytecode
    let mut bytecode_hash_chain = HashChain::default();
    for data_element in data {
        let data_felt = Felt::from_hex_unchecked(data_element);
        bytecode_hash_chain.update(data_felt);
    }
    outer.update(bytecode_hash_chain.finalize());

    Ok(outer.finalize())
}

struct HashChain {
    hash: Felt,
}

impl HashChain {
    fn new() -> Self {
        HashChain { hash: Felt::ZERO }
    }

    fn update(&mut self, felt: Felt) {
        self.hash = Poseidon::hash_array(&[self.hash, felt]);
    }

    fn finalize(self) -> Felt {
        self.hash
    }
}

impl Default for HashChain {
    fn default() -> Self {
        Self::new()
    }
}

struct PythonDefaultFormatter;

impl Formatter for PythonDefaultFormatter {
    fn begin_array<W>(&mut self, writer: &mut W) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        writer.write_all(b"[")
    }

    fn end_array<W>(&mut self, writer: &mut W) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        writer.write_all(b"]")
    }

    fn begin_array_value<W>(&mut self, writer: &mut W, first: bool) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        if !first {
            writer.write_all(b", ")?;
        }
        Ok(())
    }

    fn begin_object<W>(&mut self, writer: &mut W) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        writer.write_all(b"{")
    }

    fn end_object<W>(&mut self, writer: &mut W) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        writer.write_all(b"}")
    }

    fn begin_object_key<W>(&mut self, writer: &mut W, first: bool) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        if !first {
            writer.write_all(b", ")?;
        }
        Ok(())
    }

    fn begin_object_value<W>(&mut self, writer: &mut W) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        writer.write_all(b": ")
    }

    fn write_string_fragment<W>(&mut self, writer: &mut W, fragment: &str) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        escape_str(writer, fragment)
    }
}