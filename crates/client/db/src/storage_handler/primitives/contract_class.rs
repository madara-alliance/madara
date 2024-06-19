use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;

use anyhow::{anyhow, Context};
use blockifier::execution::contract_class::{
    ContractClass as ContractClassBlockifier, ContractClassV0, ContractClassV0Inner, ContractClassV1, EntryPointV1,
};
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
    pub contract_class: ContractClassBlockifier,
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
    pub contract: ContractClassBlockifier,
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
pub fn to_contract_class_sierra(sierra_class: &ContractClassV1, abi: String) -> anyhow::Result<ContractClassCore> {
    let entry_points_by_type: HashMap<_, _> =
        sierra_class.entry_points_by_type.iter().map(|(k, v)| (*k, v.clone())).collect();
    let entry_points_by_type = to_entry_points_by_type(&entry_points_by_type)?;

    let sierra_program = sierra_class
        .program
        .iter_data()
        .filter_map(|maybe_relocatable| {
            maybe_relocatable.get_int_ref().map(|felt| Felt::from_bytes_be(&((*felt).to_be_bytes())))
        })
        .collect::<Vec<Felt>>();

    Ok(ContractClassCore::Sierra(FlattenedSierraClass {
        sierra_program,
        contract_class_version: "0.1.0".to_string(),
        entry_points_by_type,
        abi,
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

pub fn to_contract_class_cairo(
    contract_class: &ContractClassV0,
    abi: Option<String>,
) -> anyhow::Result<ContractClassCore> {
    let serialized_program = contract_class.program.serialize().context("serializing program")?;
    let entry_points_by_type: HashMap<_, _> = contract_class.entry_points_by_type.clone().into_iter().collect();
    let serialized_entry_points = to_legacy_entry_points_by_type(&entry_points_by_type)?;

    let serialized_abi = serde_json::from_str(&abi.unwrap()).context("deserializing abi")?;

    let compressed_legacy_contract_class = CompressedLegacyContractClass {
        program: serialized_program,
        entry_points_by_type: serialized_entry_points,
        abi: serialized_abi,
    };

    Ok(ContractClassCore::Legacy(compressed_legacy_contract_class))
}

/// Converts a [LegacyContractClass] to a [ContractClassBlockifier]
pub fn from_contract_class_cairo(contract_class: &LegacyContractClass) -> anyhow::Result<ContractClassBlockifier> {
    let compressed_program = contract_class.compress().expect("Cairo program compression failed");
    let bytes = decompress(&compressed_program.program)?;
    let program = Program::from_bytes(&bytes, None).unwrap(); // check if entrypoint is needed somewhere here
    let entry_points_by_type = from_legacy_entry_points_by_type(&contract_class.entry_points_by_type);
    let blockifier_contract = ContractClassV0(Arc::new(ContractClassV0Inner { program, entry_points_by_type }));
    anyhow::Ok(ContractClassBlockifier::V0(blockifier_contract))
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

impl TryFrom<DeployedClass> for ContractClassWrapper {
    type Error = anyhow::Error;

    fn try_from(contract_class: DeployedClass) -> Result<Self, Self::Error> {
        let contract = from_rpc_contract_class(&contract_class)?;

        let abi = match &contract_class {
            DeployedClass::LegacyClass(class_cairo) => {
                let abi_string = serde_json::to_string(&class_cairo.abi).context("serializing abi")?;
                ContractAbi::Cairo(Some(abi_string))
            }
            DeployedClass::SierraClass(class_sierra) => ContractAbi::Sierra(class_sierra.abi.clone()),
        };

        let sierra_program_length = match &contract_class {
            DeployedClass::SierraClass(class_sierra) => class_sierra.sierra_program.len(),
            DeployedClass::LegacyClass(_) => 0,
        } as u64;
        let abi_length = abi.length() as u64;

        Ok(Self { contract, abi, sierra_program_length, abi_length })
    }
}

impl TryInto<ContractClassCore> for ContractClassWrapper {
    type Error = anyhow::Error;

    fn try_into(self) -> Result<ContractClassCore, Self::Error> {
        match self.contract {
            ContractClassBlockifier::V0(contract_class) => match self.abi {
                ContractAbi::Cairo(opt_string) => to_contract_class_cairo(&contract_class, opt_string),
                _ => Err(anyhow::anyhow!("Invalid ABI type for Cairo")),
            },
            ContractClassBlockifier::V1(contract_class) => match self.abi {
                ContractAbi::Sierra(string) => to_contract_class_sierra(&contract_class, string),
                _ => Err(anyhow::anyhow!("Invalid ABI type for Sierra")),
            },
        }
    }
}
