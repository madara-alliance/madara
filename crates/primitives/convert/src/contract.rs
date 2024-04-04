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
use cairo_lang_v1_0_0_alpha6::bigint::BigUintAsHex as BigUintAsHexV1;
use cairo_lang_v1_0_0_rc0::bigint::BigUintAsHex as BigUintAsHexV2;
use cairo_lang_v1_1_1::bigint::BigUintAsHex as BigUintAsHexV3;
use cairo_vm::types::program::Program;
use casm_compiler_v1_0_0_alpha6::casm_contract_class::CasmContractClass as CasmContractClassV1;
use casm_compiler_v1_0_0_alpha6::contract_class::{
    ContractClass as SierraContractClassV1, ContractEntryPoint as ContractEntryPointV1,
    ContractEntryPoints as ContractEntryPointsV1,
};
use casm_compiler_v1_0_0_rc0::casm_contract_class::CasmContractClass as CasmContractClassV2;
use casm_compiler_v1_0_0_rc0::contract_class::{
    ContractClass as SierraContractClassV2, ContractEntryPoint as ContractEntryPointV2,
    ContractEntryPoints as ContractEntryPointsV2,
};
use casm_compiler_v1_1_1::casm_contract_class::CasmContractClass as CasmContractClassV3;
use casm_compiler_v1_1_1::contract_class::{
    ContractClass as SierraContractClassV3, ContractEntryPoint as ContractEntryPointV3,
    ContractEntryPoints as ContractEntryPointsV3,
};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use indexmap::IndexMap;
use mp_felt::Felt252Wrapper;
use num_bigint::{BigInt, BigUint, Sign};
use serde_json;
use starknet_api::core::EntryPointSelector;
use starknet_api::deprecated_contract_class::{EntryPoint, EntryPointOffset, EntryPointType};
use starknet_api::hash::StarkFelt;
use starknet_core::types::{
    CompressedLegacyContractClass, ContractClass as ContractClassCore, EntryPointsByType, FieldElement,
    FlattenedSierraClass, FromByteArrayError, LegacyContractAbiEntry, LegacyContractEntryPoint,
    LegacyEntryPointsByType, SierraEntryPoint,
};

#[derive(Debug)]
pub enum CasmContractClassVersion {
    V1(CasmContractClassV1),
    V2(CasmContractClassV2),
    V3(CasmContractClassV3),
    Base(CasmContractClass),
}

#[derive(Debug)]
pub enum SierraContractClassVersion {
    V1(SierraContractClassV1),
    V2(SierraContractClassV2),
    V3(SierraContractClassV3),
    Base(SierraContractClass),
}

#[derive(Debug, PartialEq, PartialOrd)]
struct StarknetVersion(u8, u8, u8, u8);

/// Returns a [`BlockifierContractClass`] from a [`ContractClass`]
pub fn from_rpc_contract_class(
    contract_class: &ContractClassCore,
    starknet_version: Option<String>,
) -> anyhow::Result<ContractClassBlockifier> {
    log::info!("from_rpc_contract_class");
    match contract_class {
        ContractClassCore::Sierra(contract_class) => from_contract_class_sierra(contract_class, starknet_version),
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

/// Converts a [FlattenedSierraClass] to a [ContractClassBlockifier]
///
/// Note: The conversion between the different legacy class versions is handled by an intermediate
/// json representation.
pub fn from_contract_class_sierra(
    contract_class: &FlattenedSierraClass,
    starknet_version: Option<String>,
) -> anyhow::Result<ContractClassBlockifier> {
    let casm_contract = flattened_sierra_to_casm_contract_class(contract_class, starknet_version)?;
    let raw_casm_contract = match casm_contract {
        CasmContractClassVersion::V1(inner) => serde_json::to_string(&inner).unwrap(),
        CasmContractClassVersion::V2(inner) => serde_json::to_string(&inner).unwrap(),
        CasmContractClassVersion::V3(inner) => serde_json::to_string(&inner).unwrap(),
        CasmContractClassVersion::Base(inner) => serde_json::to_string(&inner).unwrap(),
    };
    let blockifier_contract = ContractClassV1::try_from_json_string(&raw_casm_contract).unwrap();
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

/// Converts a [CompressedLegacyContractClass] to a [ContractClassBlockifier]
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

/// Converts a [FlattenedSierraClass] to a [CasmContractClassVersion]
///
/// Note: The Starknet state contains different legacy versions of CasmContractClass, therefore we
/// need to convert them properly with the associated lib.
pub fn flattened_sierra_to_casm_contract_class(
    flattened_sierra: &FlattenedSierraClass,
    starknet_version: Option<String>,
) -> Result<CasmContractClassVersion, StarknetSierraCompilationError> {
    let sierra_contract_class = flattened_sierra_version(flattened_sierra, starknet_version);

    let casm_contract_class_version = match sierra_contract_class {
        SierraContractClassVersion::V1(inner) => {
            CasmContractClassVersion::V1(CasmContractClassV1::from_contract_class(inner, true).unwrap())
        }
        SierraContractClassVersion::V2(inner) => {
            CasmContractClassVersion::V2(CasmContractClassV2::from_contract_class(inner, true).unwrap())
        }
        SierraContractClassVersion::V3(inner) => {
            CasmContractClassVersion::V3(CasmContractClassV3::from_contract_class(inner, true).unwrap())
        }
        SierraContractClassVersion::Base(inner) => {
            CasmContractClassVersion::Base(CasmContractClass::from_contract_class(inner, true, usize::MAX).unwrap())
        }
    };

    Ok(casm_contract_class_version)
}

/// Converts a [FlattenedSierraClass] to a [SierraContractClassVersion]
pub fn flattened_sierra_version(
    flattened_sierra: &FlattenedSierraClass,
    starknet_version: Option<String>,
) -> SierraContractClassVersion {
    let version = get_version(&starknet_version.unwrap());

    let v1_0_0_alpha6 = StarknetVersion(0, 11, 0, 0);
    let v1_0_0_rc0 = StarknetVersion(0, 11, 0, 0);
    let v1_1_1 = StarknetVersion(0, 11, 1, 0);

    let sierra_contract_class = if version <= v1_0_0_alpha6 {
        SierraContractClassVersion::V1(SierraContractClassV1 {
            sierra_program: flattened_sierra.sierra_program.iter().map(field_element_to_big_uint_as_hex_v1).collect(),
            sierra_program_debug_info: None,
            contract_class_version: flattened_sierra.contract_class_version.clone(),
            entry_points_by_type: entry_points_by_type_to_contract_entry_points_v1(
                flattened_sierra.entry_points_by_type.clone(),
            ),
            abi: None, // TODO: Pass the abi correctly on sierra contract classes
        })
    } else if version > v1_0_0_rc0 {
        SierraContractClassVersion::V2(SierraContractClassV2 {
            sierra_program: flattened_sierra.sierra_program.iter().map(field_element_to_big_uint_as_hex_v2).collect(),
            sierra_program_debug_info: None,
            contract_class_version: flattened_sierra.contract_class_version.clone(),
            entry_points_by_type: entry_points_by_type_to_contract_entry_points_v2(
                flattened_sierra.entry_points_by_type.clone(),
            ),
            abi: None, // TODO: Pass the abi correctly on sierra contract classes
        })
    } else if version > v1_1_1 {
        SierraContractClassVersion::V3(SierraContractClassV3 {
            sierra_program: flattened_sierra.sierra_program.iter().map(field_element_to_big_uint_as_hex_v3).collect(),
            sierra_program_debug_info: None,
            contract_class_version: flattened_sierra.contract_class_version.clone(),
            entry_points_by_type: entry_points_by_type_to_contract_entry_points_v3(
                flattened_sierra.entry_points_by_type.clone(),
            ),
            abi: None, // TODO: Pass the abi correctly on sierra contract classes
        })
    } else {
        SierraContractClassVersion::Base(SierraContractClass {
            sierra_program: flattened_sierra.sierra_program.iter().map(field_element_to_big_uint_as_hex).collect(),
            sierra_program_debug_info: None,
            contract_class_version: flattened_sierra.contract_class_version.clone(),
            entry_points_by_type: entry_points_by_type_to_contract_entry_points(
                flattened_sierra.entry_points_by_type.clone(),
            ),
            abi: None, // TODO: Pass the abi correctly on sierra contract classes
        })
    };

    sierra_contract_class
}

/// Converts a [Arc<FlattenedSierraClass>] to a [starknet_api::state::ContractClass]
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

/// Converts a [EntryPointsByType] to a [ContractEntryPoints]
fn entry_points_by_type_to_contract_entry_points_v1(value: EntryPointsByType) -> ContractEntryPointsV1 {
    fn sierra_entry_point_to_contract_entry_point(value: SierraEntryPoint) -> ContractEntryPointV1 {
        ContractEntryPointV1 {
            function_idx: value.function_idx.try_into().unwrap(),
            selector: field_element_to_big_uint(&value.selector),
        }
    }
    ContractEntryPointsV1 {
        constructor: value.constructor.iter().map(|x| sierra_entry_point_to_contract_entry_point(x.clone())).collect(),
        external: value.external.iter().map(|x| sierra_entry_point_to_contract_entry_point(x.clone())).collect(),
        l1_handler: value.l1_handler.iter().map(|x| sierra_entry_point_to_contract_entry_point(x.clone())).collect(),
    }
}

/// Converts a [EntryPointsByType] to a [ContractEntryPoints]
fn entry_points_by_type_to_contract_entry_points_v2(value: EntryPointsByType) -> ContractEntryPointsV2 {
    fn sierra_entry_point_to_contract_entry_point(value: SierraEntryPoint) -> ContractEntryPointV2 {
        ContractEntryPointV2 {
            function_idx: value.function_idx.try_into().unwrap(),
            selector: field_element_to_big_uint(&value.selector),
        }
    }
    ContractEntryPointsV2 {
        constructor: value.constructor.iter().map(|x| sierra_entry_point_to_contract_entry_point(x.clone())).collect(),
        external: value.external.iter().map(|x| sierra_entry_point_to_contract_entry_point(x.clone())).collect(),
        l1_handler: value.l1_handler.iter().map(|x| sierra_entry_point_to_contract_entry_point(x.clone())).collect(),
    }
}

/// Converts a [EntryPointsByType] to a [ContractEntryPoints]
fn entry_points_by_type_to_contract_entry_points_v3(value: EntryPointsByType) -> ContractEntryPointsV3 {
    fn sierra_entry_point_to_contract_entry_point(value: SierraEntryPoint) -> ContractEntryPointV3 {
        ContractEntryPointV3 {
            function_idx: value.function_idx.try_into().unwrap(),
            selector: field_element_to_big_uint(&value.selector),
        }
    }
    ContractEntryPointsV3 {
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
    if value == &FieldElement::ZERO {
        BigInt::from_bytes_be(Sign::NoSign, &value.to_bytes_be()).to_biguint().unwrap()
    } else {
        BigInt::from_bytes_be(Sign::Plus, &value.to_bytes_be()).to_biguint().unwrap()
    }
}

/// Converts a [FieldElement] to a [BigUintAsHexV1]
fn field_element_to_big_uint_as_hex_v1(value: &FieldElement) -> BigUintAsHexV1 {
    BigUintAsHexV1 { value: field_element_to_big_uint(value) }
}

/// Converts a [FieldElement] to a [BigUintAsHexV2]
fn field_element_to_big_uint_as_hex_v2(value: &FieldElement) -> BigUintAsHexV2 {
    BigUintAsHexV2 { value: field_element_to_big_uint(value) }
}

/// Converts a [FieldElement] to a [BigUintAsHexV3]
fn field_element_to_big_uint_as_hex_v3(value: &FieldElement) -> BigUintAsHexV3 {
    BigUintAsHexV3 { value: field_element_to_big_uint(value) }
}

/// Converts a [FieldElement] to a [BigUintAsHex]
fn field_element_to_big_uint_as_hex(value: &FieldElement) -> BigUintAsHex {
    BigUintAsHex { value: field_element_to_big_uint(value) }
}

/// Returns a [StarknetVersion] from a version string
fn get_version(version_str: &str) -> StarknetVersion {
    let parts: Vec<u8> = version_str.split('.').map(|part| part.parse::<u8>().unwrap_or(0)).collect();

    let mut parts_padded = parts.clone();
    while parts_padded.len() < 3 {
        parts_padded.push(0);
    }

    StarknetVersion(parts_padded[0], parts_padded[1], parts_padded[2], 0)
}
