use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;

use anyhow::{anyhow, Context};
use blockifier::execution::contract_class::{
    deserialize_program, ContractClass as ContractClassBlockifier, ContractClassV0, ContractClassV0Inner,
    ContractClassV1, EntryPointV1,
};
use cairo_vm::serde::deserialize_program::{parse_program_json, ProgramJson};
use cairo_vm::types::program::Program;
use dp_convert::to_felt::ToFelt;
use dp_transactions::from_broadcasted_transactions::flattened_sierra_to_casm_contract_class;
use flate2::read::GzDecoder;
use indexmap::IndexMap;
use parity_scale_codec::{Decode, Encode};
use starknet_api::core::{ClassHash, EntryPointSelector, Nonce};
use starknet_api::deprecated_contract_class::{EntryPoint, EntryPointOffset, EntryPointType};
use starknet_api::hash::StarkFelt;
use starknet_core::types::contract::legacy::{LegacyContractClass, RawLegacyEntryPoint, RawLegacyEntryPoints};
use starknet_core::types::{
    CompressedLegacyContractClass, ContractClass as ContractClassCore, EntryPointsByType, FlattenedSierraClass,
    LegacyContractEntryPoint, LegacyEntryPointsByType, SierraEntryPoint,
};

#[derive(Debug, Encode, Decode)]
pub struct StorageContractClassData {
    pub contract_class: String,
    pub abi: ContractAbi,
    pub sierra_program_length: u64,
    pub abi_length: u64,
    pub block_number: u64,
}
#[derive(Debug, Clone, Encode, Decode)]
pub struct StorageContractData {
    pub class_hash: ClassHash,
    pub nonce: Nonce,
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct ClassUpdateWrapper(pub Vec<ContractClassData>);
#[derive(Debug, Clone, Encode, Decode)]
pub struct ContractClassData {
    pub hash: ClassHash,
    pub contract_class: ContractClassWrapper,
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct ContractClassWrapper {
    pub contract_class: String,
    pub abi: ContractAbi,
    pub sierra_program_length: u64,
    pub abi_length: u64,
}
// TODO: move this somewhere more sensible? Would be a good idea to decouple
// publicly available storage data from wrapper classes
#[derive(Debug, Clone, Encode, Decode)]
pub enum ContractAbi {
    Sierra(String),
    Cairo(Option<String>),
}

impl ContractAbi {
    pub fn length(&self) -> usize {
        match self {
            ContractAbi::Sierra(abi) => abi.len(),
            ContractAbi::Cairo(Some(entries)) => entries.len(),
            ContractAbi::Cairo(None) => 0,
        }
    }
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct AbiEventEntryWrapper {
    /// The event name
    pub name: String,
    /// Typed parameter
    pub keys: Vec<AbiTypedParameterWrapper>,
    /// Typed parameter
    pub data: Vec<AbiTypedParameterWrapper>,
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct AbiStructEntryWrapper {
    pub name: String,
    pub size: u64,
    pub members: Vec<AbiStructMemberWrapper>,
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct AbiStructMemberWrapper {
    /// The parameter's name
    pub name: String,
    /// The parameter's type
    pub r#type: String,
    /// Offset of this property within the struct
    pub offset: u64,
}

#[derive(Debug, Clone, Encode, Decode)]
pub enum AbiFunctionTypeWrapper {
    Function,
    L1handler,
    Constructor,
}

#[derive(Debug, Clone, Encode, Decode)]
pub enum AbiEventTypeWrapper {
    Event,
}

#[derive(Debug, Clone, Encode, Decode)]
pub enum AbiStructTypeWrapper {
    Struct,
}

#[derive(Debug, Clone, Encode, Decode)]
pub enum AbiFunctionStateMutabilityWrapper {
    View,
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct AbiTypedParameterWrapper {
    pub name: String,
    pub r#type: String,
}

pub fn from_rpc_contract_class(contract_class: &DeployedClass) -> anyhow::Result<ContractClassBlockifier> {
    match contract_class {
        DeployedClass::LegacyClass(contract_class) => from_contract_class_cairo(contract_class),
        DeployedClass::SierraClass(contract_class) => from_contract_class_sierra(contract_class),
    }
}

/// Converts a [ContractClassBlockifier] to a [ContractClassCore]
///
/// This is after extracting the contract classes.
pub fn to_contract_class_sierra(json_str: &str, abi: String) -> anyhow::Result<ContractClassCore> {
    let mut sierra_class: FlattenedSierraClass = serde_json::from_str(json_str)?;
    sierra_class.abi = abi;

    Ok(ContractClassCore::Sierra(FlattenedSierraClass {
        sierra_program: sierra_class.sierra_program,
        abi: sierra_class.abi,
        contract_class_version: sierra_class.contract_class_version,
        entry_points_by_type: sierra_class.entry_points_by_type,
    }))
}

/// Converts a [FlattenedSierraClass] to a [ContractClassBlockifier]
///
/// This is used before storing the contract classes.
pub fn from_contract_class_sierra(contract_class: &FlattenedSierraClass) -> anyhow::Result<ContractClassBlockifier> {
    let raw_casm_contract = flattened_sierra_to_casm_contract_class(&Arc::new(contract_class.clone()))?;
    let blockifier_contract = ContractClassV1::try_from(raw_casm_contract).unwrap();
    anyhow::Ok(ContractClassBlockifier::V1(blockifier_contract))
}

/// Converts a [ContractClassBlockifier] to a [ContractClassCore]
///
/// This is after extracting the contract classes.
pub fn to_contract_class_cairo(json_str: &str, abi: Option<String>) -> anyhow::Result<ContractClassCore> {
    log::info!("icci");
    let cairo_class: LegacyContractClass = serde_json::from_str(json_str)?;
    let compressed_cairo_class = cairo_class.compress().expect("Cairo program compression failed");

    Ok(ContractClassCore::Legacy(CompressedLegacyContractClass {
        program: compressed_cairo_class.program,
        abi: None,
        entry_points_by_type: compressed_cairo_class.entry_points_by_type,
    }))
}

/// Converts a [LegacyContractClass] to a [ContractClassBlockifier]
pub fn from_contract_class_cairo(contract_class: &LegacyContractClass) -> anyhow::Result<ContractClassBlockifier> {
    let compressed_program = contract_class.compress().expect("Cairo program compression failed");

    let mut d = GzDecoder::new(&compressed_program.program[..]);
    let mut decompressed_program = Vec::new();
    d.read_to_end(&mut decompressed_program).context("Decompressing program failed")?;

    let program = Program::from_bytes(&decompressed_program, None).context("Deserializing program")?;

    let entry_points_by_type = from_legacy_entry_points_by_type(&contract_class.entry_points_by_type);
    let blockifier_contract = ContractClassV0(Arc::new(ContractClassV0Inner { program, entry_points_by_type }));
    Ok(ContractClassBlockifier::V0(blockifier_contract))
}

/// Decompresses a compressed json string into it's byte representation.
/// Example compression from [Starknet-rs](https://github.com/xJonathanLEI/starknet-rs/blob/49719f49a18f9621fc37342959e84900b600083e/starknet-core/src/types/contract/legacy.rs#L473)
pub fn decompress(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    // Program is expected to be a gzip-compressed then base64 encoded
    // representation of the JSON.
    let mut gzip_encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    serde_json::to_writer(&mut gzip_encoder, &data).context("Compressing program")?;
    let compressed_program = gzip_encoder.finish().context("Finalizing program compression")?;

    anyhow::Ok(compressed_program)
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
            .collect())
    }

    let constructor = collect_entry_points(entries, EntryPointType::Constructor).unwrap_or_default();
    let external = collect_entry_points(entries, EntryPointType::External)?;
    let l1_handler = collect_entry_points(entries, EntryPointType::L1Handler).unwrap_or_default();

    Ok(LegacyEntryPointsByType { constructor, external, l1_handler })
}

/// Returns a [anyhow::Result<LegacyEntryPointsByType>] (starknet-rs type) from
/// a [HashMap<EntryPointType, Vec<EntryPoinV1>>]
fn to_entry_points_by_type(entries: &HashMap<EntryPointType, Vec<EntryPointV1>>) -> anyhow::Result<EntryPointsByType> {
    fn collect_entry_points(
        entries: &HashMap<EntryPointType, Vec<EntryPointV1>>,
        entry_point_type: EntryPointType,
    ) -> anyhow::Result<Vec<SierraEntryPoint>> {
        Ok(entries
            .get(&entry_point_type)
            .ok_or(anyhow!("Missing {:?} entry point", entry_point_type))?
            .iter()
            .enumerate()
            .map(|(index, e)| to_entry_point(e.clone(), index as u64))
            .collect())
    }

    let constructor = collect_entry_points(entries, EntryPointType::Constructor).unwrap_or_default();
    let external = collect_entry_points(entries, EntryPointType::External)?;
    let l1_handler = collect_entry_points(entries, EntryPointType::L1Handler).unwrap_or_default();

    Ok(EntryPointsByType { constructor, external, l1_handler })
}

/// Returns a [IndexMap<EntryPointType, Vec<EntryPoint>>] from a
/// [LegacyEntryPointsByType]
fn from_legacy_entry_points_by_type(entries: &RawLegacyEntryPoints) -> IndexMap<EntryPointType, Vec<EntryPoint>> {
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
fn to_legacy_entry_point(entry_point: EntryPoint) -> LegacyContractEntryPoint {
    let selector = entry_point.selector.0.to_felt();
    let offset = entry_point.offset.0;
    LegacyContractEntryPoint { selector, offset }
}

/// Returns a [SierraEntryPoint] (starknet-rs) from a [EntryPointV1]
/// (starknet-api)
fn to_entry_point(entry_point: EntryPointV1, index: u64) -> SierraEntryPoint {
    let selector = entry_point.selector.0.to_felt();
    let function_idx = index;
    SierraEntryPoint { selector, function_idx }
}

/// Returns a [EntryPoint] (starknet-api) from a [LegacyContractEntryPoint]
/// (starknet-rs)
fn from_legacy_entry_point(entry_point: &RawLegacyEntryPoint) -> EntryPoint {
    let selector = EntryPointSelector(StarkFelt::new_unchecked(entry_point.selector.to_bytes_be()));
    let offset = EntryPointOffset(entry_point.offset.into());
    EntryPoint { selector, offset }
}

use starknet_providers::sequencer::models::DeployedClass;
use starknet_types_core::felt::Felt;

// Wrapper Class conversion

impl TryFrom<serde_json::Value> for ContractClassWrapper {
    type Error = anyhow::Error;

    fn try_from(contract_class: serde_json::Value) -> Result<Self, Self::Error> {
        let class = contract_class.to_string();

        // Use Option for abi and sierra_program to handle missing cases
        let abi_value = contract_class.get("abi");

        let sierra_program = contract_class.get("sierra_program");

        let sierra_program_length = match sierra_program {
            Some(serde_json::Value::Array(program)) => program.len() as u64,
            _ => 0, // Set length to 0 if sierra_program is missing or not an array
        };

        let abi = if sierra_program_length > 0 {
            match abi_value {
                Some(abi_value) => {
                    let abi_string =
                        abi_value.as_str().ok_or_else(|| anyhow::anyhow!("Invalid `abi` field format"))?.to_string();
                    ContractAbi::Sierra(abi_string)
                }
                None => return Err(anyhow::anyhow!("Missing `abi` field for Sierra program")), // Handle missing abi when sierra_program is present
            }
        } else {
            match abi_value {
                Some(abi_value) => {
                    let abi_string = serde_json::to_string(abi_value).context("serializing abi")?;
                    ContractAbi::Cairo(Some(abi_string))
                }
                None => ContractAbi::Cairo(None),
            }
        };

        let abi_length = match &abi {
            ContractAbi::Cairo(Some(abi_string)) => abi_string.len() as u64,
            ContractAbi::Cairo(None) => 0,
            ContractAbi::Sierra(abi_string) => abi_string.len() as u64,
        };

        Ok(Self { contract_class: class, abi, sierra_program_length, abi_length })
    }
}

impl TryInto<ContractClassCore> for ContractClassWrapper {
    type Error = anyhow::Error;

    fn try_into(self) -> Result<ContractClassCore, Self::Error> {
        if self.sierra_program_length > 0 {
            match self.abi {
                ContractAbi::Sierra(abi_string) => to_contract_class_sierra(&self.contract_class, abi_string),
                _ => Err(anyhow::anyhow!("Invalid ABI type for Sierra")),
            }
        } else {
            match self.abi {
                ContractAbi::Cairo(opt_string) => to_contract_class_cairo(&self.contract_class, opt_string),
                _ => Err(anyhow::anyhow!("Invalid ABI type for Cairo")),
            }
        }
    }
}
