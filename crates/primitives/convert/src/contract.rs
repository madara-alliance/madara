use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Arc;

use anyhow::anyhow;
use blockifier::execution::contract_class::{
    ContractClass as ContractClassBlockifier, ContractClassV0, ContractClassV0Inner, ContractClassV1,
};
use cairo_lang_starknet_classes::casm_contract_class::{CasmContractClass, StarknetSierraCompilationError};
use cairo_lang_starknet_classes::contract_class::{
    ContractClass as SierraContractClass, ContractEntryPoint, ContractEntryPoints,
};
use cairo_lang_utils::bigint::BigUintAsHex;
use cairo_vm::types::program::Program;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use indexmap::IndexMap;
use mp_felt::Felt252Wrapper;
use num_bigint::{BigInt, BigUint, Sign};
use starknet_api::core::EntryPointSelector;
use starknet_api::deprecated_contract_class::{EntryPoint, EntryPointOffset, EntryPointType};
use starknet_api::hash::StarkFelt;
use starknet_core::types::{
    CompressedLegacyContractClass, ContractClass as ContractClassCore, EntryPointsByType, FieldElement,
    FlattenedSierraClass, FromByteArrayError, LegacyContractAbiEntry, LegacyContractEntryPoint,
    LegacyEntryPointsByType, SierraEntryPoint,
};

/// Returns a [`BlockifierContractClass`] from a [`ContractClass`]
pub fn from_rpc_contract_class(contract_class: &ContractClassCore) -> anyhow::Result<ContractClassBlockifier> {
    match contract_class {
        ContractClassCore::Sierra(contract_class) => from_contract_class_sierra(contract_class),
        ContractClassCore::Legacy(contract_class) => from_contract_class_cairo(contract_class),
    }
}

// TODO: implement this
pub fn to_contract_class_sierra(_: &ContractClassV1, abi: String) -> anyhow::Result<ContractClassCore> {
    Ok(ContractClassCore::Sierra(FlattenedSierraClass {
        sierra_program: vec![], // FIXME: https://github.com/keep-starknet-strange/madara/issues/775
        contract_class_version: option_env!("COMPILER_VERSION").unwrap_or("0.11.2").into(),
        entry_points_by_type: EntryPointsByType { constructor: vec![], external: vec![], l1_handler: vec![] }, /* TODO: add entry_points_by_type */
        abi,
    }))
}

pub fn from_contract_class_sierra(contract_class: &FlattenedSierraClass) -> anyhow::Result<ContractClassBlockifier> {
    let casm_contract = flattened_sierra_to_casm_contract_class(contract_class)?;
    let blockifier_contract = ContractClassV1::try_from(casm_contract).unwrap();
    anyhow::Ok(ContractClassBlockifier::V1(blockifier_contract))
}

pub fn to_contract_class_cairo(
    contract_class: &ContractClassV0,
    abi: Option<Vec<LegacyContractAbiEntry>>,
) -> anyhow::Result<ContractClassCore> {
    let entry_points_by_type: HashMap<_, _> =
        contract_class.entry_points_by_type.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let entry_points_by_type = to_legacy_entry_points_by_type(&entry_points_by_type)?;
    let compressed_program = compress(&contract_class.program.serialize()?)?;
    Ok(ContractClassCore::Legacy(CompressedLegacyContractClass {
        program: compressed_program,
        entry_points_by_type,
        abi,
    }))
}

pub fn from_contract_class_cairo(
    contract_class: &CompressedLegacyContractClass,
) -> anyhow::Result<ContractClassBlockifier> {
    // decompressed program into json string bytes, then serialize bytes into
    // Program this can cause issues depending on the format used by
    // cairo-vm during deserialization
    let bytes = decompress(&contract_class.program)?;
    let program = Program::from_bytes(&bytes, None).unwrap(); // FIXME: Problems in deserializing program JSON
    let entry_points_by_type = from_legacy_entry_points_by_type(&contract_class.entry_points_by_type);
    let blockifier_contract = ContractClassV0(Arc::new(ContractClassV0Inner { program, entry_points_by_type }));
    anyhow::Ok(ContractClassBlockifier::V0(blockifier_contract))
}

/// Converts a [FlattenedSierraClass] to a [CasmContractClass]
pub fn flattened_sierra_to_casm_contract_class(
    flattened_sierra: &FlattenedSierraClass,
) -> Result<CasmContractClass, StarknetSierraCompilationError> {
    let sierra_contract_class = SierraContractClass {
        sierra_program: flattened_sierra.sierra_program.iter().map(field_element_to_big_uint_as_hex).collect(),
        sierra_program_debug_info: None,
        contract_class_version: flattened_sierra.contract_class_version.clone(),
        entry_points_by_type: entry_points_by_type_to_contract_entry_points(
            flattened_sierra.entry_points_by_type.clone(),
        ),
        abi: None, // we can convert the ABI but for now, to convert to Casm, the ABI isn't needed
    };
    // TODO : fix from_contract_class parameters here
    let casm_contract_class = CasmContractClass::from_contract_class(sierra_contract_class, false, usize::MAX).unwrap();
    Ok(casm_contract_class)
}

pub fn flattened_sierra_to_sierra_contract_class(
    flattened_sierra: Arc<FlattenedSierraClass>,
) -> starknet_api::state::ContractClass {
    let mut entry_points_by_type =
        IndexMap::<starknet_api::state::EntryPointType, Vec<starknet_api::state::EntryPoint>>::with_capacity(3);
    for sierra_entrypoint in flattened_sierra.entry_points_by_type.constructor.iter() {
        entry_points_by_type
            .entry(starknet_api::state::EntryPointType::Constructor)
            .or_default()
            .push(rpc_entry_point_to_starknet_api_entry_point(sierra_entrypoint));
    }
    for sierra_entrypoint in flattened_sierra.entry_points_by_type.external.iter() {
        entry_points_by_type
            .entry(starknet_api::state::EntryPointType::External)
            .or_default()
            .push(rpc_entry_point_to_starknet_api_entry_point(sierra_entrypoint));
    }
    for sierra_entrypoint in flattened_sierra.entry_points_by_type.l1_handler.iter() {
        entry_points_by_type
            .entry(starknet_api::state::EntryPointType::L1Handler)
            .or_default()
            .push(rpc_entry_point_to_starknet_api_entry_point(sierra_entrypoint));
    }
    starknet_api::state::ContractClass {
        sierra_program: flattened_sierra.sierra_program.iter().map(|f| Felt252Wrapper(*f).into()).collect(),
        entry_points_by_type,
        abi: flattened_sierra.abi.clone(),
    }
}

/// Converts a [EntryPointsByType] to a [ContractEntryPoints]
fn entry_points_by_type_to_contract_entry_points(value: EntryPointsByType) -> ContractEntryPoints {
    fn sierra_entry_point_to_contract_entry_point(value: SierraEntryPoint) -> ContractEntryPoint {
        ContractEntryPoint {
            function_idx: value.function_idx.try_into().unwrap(),
            selector: field_element_to_big_uint(&value.selector),
        }
    }
    ContractEntryPoints {
        constructor: value.constructor.iter().map(|x| sierra_entry_point_to_contract_entry_point(x.clone())).collect(),
        external: value.external.iter().map(|x| sierra_entry_point_to_contract_entry_point(x.clone())).collect(),
        l1_handler: value.l1_handler.iter().map(|x| sierra_entry_point_to_contract_entry_point(x.clone())).collect(),
    }
}

fn rpc_entry_point_to_starknet_api_entry_point(value: &SierraEntryPoint) -> starknet_api::state::EntryPoint {
    starknet_api::state::EntryPoint {
        function_idx: starknet_api::state::FunctionIndex(value.function_idx),
        selector: Felt252Wrapper(value.selector).into(),
    }
}

/// Returns a compressed vector of bytes
pub(crate) fn compress(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut gzip_encoder = GzEncoder::new(Vec::new(), flate2::Compression::fast());
    // 2023-08-22: JSON serialization is already done in Blockifier
    // https://github.com/keep-starknet-strange/blockifier/blob/no_std-support-7578442/crates/blockifier/src/execution/contract_class.rs#L129
    // https://github.com/keep-starknet-strange/blockifier/blob/no_std-support-7578442/crates/blockifier/src/execution/contract_class.rs#L389
    // serde_json::to_writer(&mut gzip_encoder, data)?;
    gzip_encoder.write_all(data)?;
    Ok(gzip_encoder.finish()?)
}

/// Decompresses a compressed json string into it's byte representation.
/// Example compression from [Starknet-rs](https://github.com/xJonathanLEI/starknet-rs/blob/49719f49a18f9621fc37342959e84900b600083e/starknet-core/src/types/contract/legacy.rs#L473)
pub(crate) fn decompress(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut gzip_decoder = GzDecoder::new(data);
    let mut buf = Vec::<u8>::new();
    gzip_decoder.read_to_end(&mut buf)?;
    anyhow::Ok(buf)
}

/// Returns a [anyhow::Result<LegacyEntryPointsByType>] (starknet-rs type) from
/// a [HashMap<EntryPointType, Vec<EntryPoint>>]
fn to_legacy_entry_points_by_type(
    entries: &HashMap<EntryPointType, Vec<EntryPoint>>,
) -> anyhow::Result<LegacyEntryPointsByType> {
    fn collect_entry_points(
        entries: &HashMap<EntryPointType, Vec<EntryPoint>>,
        entry_point_type: EntryPointType,
    ) -> anyhow::Result<Vec<LegacyContractEntryPoint>> {
        Ok(entries
            .get(&entry_point_type)
            .ok_or(anyhow!("Missing {:?} entry point", entry_point_type))?
            .iter()
            .map(|e| to_legacy_entry_point(e.clone()))
            .collect::<Result<Vec<LegacyContractEntryPoint>, FromByteArrayError>>()?)
    }

    let constructor = collect_entry_points(entries, EntryPointType::Constructor).unwrap_or_default();
    let external = collect_entry_points(entries, EntryPointType::External)?;
    let l1_handler = collect_entry_points(entries, EntryPointType::L1Handler).unwrap_or_default();

    Ok(LegacyEntryPointsByType { constructor, external, l1_handler })
}

/// Returns a [IndexMap<EntryPointType, Vec<EntryPoint>>] from a
/// [LegacyEntryPointsByType]
fn from_legacy_entry_points_by_type(entries: &LegacyEntryPointsByType) -> IndexMap<EntryPointType, Vec<EntryPoint>> {
    core::iter::empty()
        .chain(entries.constructor.iter().map(|entry| (EntryPointType::Constructor, entry)))
        .chain(entries.external.iter().map(|entry| (EntryPointType::External, entry)))
        .chain(entries.l1_handler.iter().map(|entry| (EntryPointType::L1Handler, entry)))
        .fold(IndexMap::new(), |mut map, (entry_type, entry)| {
            map.entry(entry_type).or_default().push(from_legacy_entry_point(entry));
            map
        })
}

/// Returns a [LegacyContractEntryPoint] (starknet-rs) from a [EntryPoint]
/// (starknet-api)
fn to_legacy_entry_point(entry_point: EntryPoint) -> Result<LegacyContractEntryPoint, FromByteArrayError> {
    let selector = FieldElement::from_bytes_be(&entry_point.selector.0.0)?;
    let offset = entry_point.offset.0 as u64;
    Ok(LegacyContractEntryPoint { selector, offset })
}

/// Returns a [EntryPoint] (starknet-api) from a [LegacyContractEntryPoint]
/// (starknet-rs)
fn from_legacy_entry_point(entry_point: &LegacyContractEntryPoint) -> EntryPoint {
    let selector = EntryPointSelector(StarkFelt(entry_point.selector.to_bytes_be()));
    let offset = EntryPointOffset(entry_point.offset);
    EntryPoint { selector, offset }
}

/// Converts a [FieldElement] to a [BigUint]
fn field_element_to_big_uint(value: &FieldElement) -> BigUint {
    BigInt::from_bytes_be(Sign::Plus, &value.to_bytes_be()).to_biguint().unwrap()
}

/// Converts a [FieldElement] to a [BigUintAsHex]
fn field_element_to_big_uint_as_hex(value: &FieldElement) -> BigUintAsHex {
    BigUintAsHex { value: field_element_to_big_uint(value) }
}
