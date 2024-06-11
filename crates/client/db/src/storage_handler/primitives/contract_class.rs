use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Arc;

use anyhow::anyhow;
use blockifier::execution::contract_class::{
    ContractClass as ContractClassBlockifier, ContractClassV0, ContractClassV0Inner, ContractClassV1, EntryPointV1,
};
use cairo_vm::types::program::Program;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use mp_felt::Felt252Wrapper;
use mp_transactions::from_broadcasted_transactions::flattened_sierra_to_casm_contract_class;
use serde::{Deserialize, Serialize};
use starknet_api::core::{ClassHash, EntryPointSelector, Nonce};
use starknet_api::deprecated_contract_class::{EntryPoint, EntryPointOffset, EntryPointType};
use starknet_api::hash::StarkFelt;
use starknet_core::types::{
    CompressedLegacyContractClass, ContractClass as ContractClassCore, EntryPointsByType, FlattenedSierraClass,
    FromByteArrayError, LegacyContractAbiEntry, LegacyContractEntryPoint, LegacyEntryPointsByType, SierraEntryPoint,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct StorageContractClassData {
    pub contract_class: ContractClassBlockifier,
    pub abi: ContractAbi,
    pub sierra_program_length: u64,
    pub abi_length: u64,
    pub block_number: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageContractData {
    pub class_hash: ClassHash,
    pub nonce: Nonce,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClassUpdateWrapper(pub Vec<ContractClassData>);

#[derive(Debug, Serialize, Deserialize)]
pub struct ContractClassData {
    pub hash: ClassHash,
    pub contract_class: ContractClassWrapper,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContractClassWrapper {
    pub contract: ContractClassBlockifier,
    pub abi: ContractAbi,
    pub sierra_program_length: u64,
    pub abi_length: u64,
}
// TODO: move this somewhere more sensible? Would be a good idea to decouple
// publicly available storage data from wrapper classes
#[derive(Debug, Serialize, Deserialize)]
pub enum ContractAbi {
    Sierra(String),
    Cairo(Option<Vec<AbiEntryWrapper>>),
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

#[derive(Debug, Serialize, Deserialize)]
pub enum AbiEntryWrapper {
    Function(AbiFunctionEntryWrapper),
    Event(AbiEventEntryWrapper),
    Struct(AbiStructEntryWrapper),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AbiFunctionEntryWrapper {
    // Function abi type
    pub r#type: AbiFunctionTypeWrapper,
    /// The function name
    pub name: String,
    /// Typed parameter
    pub inputs: Vec<AbiTypedParameterWrapper>,
    /// Typed parameter
    pub outputs: Vec<AbiTypedParameterWrapper>,
    /// Function state mutability
    pub state_mutability: Option<AbiFunctionStateMutabilityWrapper>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AbiEventEntryWrapper {
    /// Event abi type
    pub r#type: AbiEventTypeWrapper,
    /// The event name
    pub name: String,
    /// Typed parameter
    pub keys: Vec<AbiTypedParameterWrapper>,
    /// Typed parameter
    pub data: Vec<AbiTypedParameterWrapper>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AbiStructEntryWrapper {
    pub r#type: AbiStructTypeWrapper,
    pub name: String,
    pub size: u64,
    pub members: Vec<AbiStructMemberWrapper>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AbiStructMemberWrapper {
    /// The parameter's name
    pub name: String,
    /// The parameter's type
    pub r#type: String,
    /// Offset of this property within the struct
    pub offset: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum AbiFunctionTypeWrapper {
    Function,
    L1handler,
    Constructor,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum AbiEventTypeWrapper {
    Event,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum AbiStructTypeWrapper {
    Struct,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum AbiFunctionStateMutabilityWrapper {
    View,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AbiTypedParameterWrapper {
    pub name: String,
    pub r#type: String,
}

/// Returns a [`BlockifierContractClass`] from a [`ContractClass`]
pub fn from_rpc_contract_class(contract_class: ContractClassCore) -> anyhow::Result<ContractClassBlockifier> {
    match contract_class {
        ContractClassCore::Legacy(contract_class) => from_contract_class_cairo(contract_class),
        ContractClassCore::Sierra(contract_class) => from_contract_class_sierra(contract_class),
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
            maybe_relocatable.get_int_ref().map(|felt| Felt252Wrapper::from((*felt).clone()))
        })
        .collect::<Vec<Felt252Wrapper>>();

    Ok(ContractClassCore::Sierra(FlattenedSierraClass {
        sierra_program: sierra_program.into_iter().map(Into::into).collect(),
        contract_class_version: "0.1.0".to_string(),
        entry_points_by_type,
        abi,
    }))
}

/// Converts a [FlattenedSierraClass] to a [ContractClassBlockifier]
///
/// This is used before storing the contract classes.
pub fn from_contract_class_sierra(contract_class: FlattenedSierraClass) -> anyhow::Result<ContractClassBlockifier> {
    let raw_casm_contract = flattened_sierra_to_casm_contract_class(&Arc::new(contract_class))?;
    let blockifier_contract = ContractClassV1::try_from(raw_casm_contract).unwrap();
    anyhow::Ok(ContractClassBlockifier::V1(blockifier_contract))
}

pub fn to_contract_class_cairo(
    contract_class: &ContractClassV0,
    abi: Option<Vec<LegacyContractAbiEntry>>,
) -> anyhow::Result<ContractClassCore> {
    let entry_points_by_type: HashMap<_, _> =
        contract_class.entry_points_by_type.iter().map(|(k, v)| (*k, v.clone())).collect();
    let entry_points_by_type = to_legacy_entry_points_by_type(&entry_points_by_type)?;
    // Here we order the program passing it to a wrapper of BTreeMaps in order to always obtain the same
    // result due to HashMap disruptions
    // let ordered_program = order_program(&contract_class.program);
    let compressed_program = compress(&contract_class.program.serialize()?)?;
    let encoded_program = base64::encode(compressed_program);
    Ok(ContractClassCore::Legacy(CompressedLegacyContractClass {
        program: encoded_program.into(),
        entry_points_by_type,
        abi,
    }))
}

/// Converts a [CompressedLegacyContractClass] to a [ContractClassBlockifier]
pub fn from_contract_class_cairo(
    contract_class: CompressedLegacyContractClass,
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
            .map(|(index, e)| to_entry_point(e.clone(), index.try_into().unwrap()))
            .collect::<Result<Vec<SierraEntryPoint>, FromByteArrayError>>()?)
    }

    let constructor = collect_entry_points(entries, EntryPointType::Constructor).unwrap_or_default();
    let external = collect_entry_points(entries, EntryPointType::External)?;
    let l1_handler = collect_entry_points(entries, EntryPointType::L1Handler).unwrap_or_default();

    Ok(EntryPointsByType { constructor, external, l1_handler })
}

/// Returns a [IndexMap<EntryPointType, Vec<EntryPoint>>] from a
/// [LegacyEntryPointsByType]
fn from_legacy_entry_points_by_type(entries: &LegacyEntryPointsByType) -> HashMap<EntryPointType, Vec<EntryPoint>> {
    core::iter::empty()
        .chain(entries.constructor.iter().map(|entry| (EntryPointType::Constructor, entry)))
        .chain(entries.external.iter().map(|entry| (EntryPointType::External, entry)))
        .chain(entries.l1_handler.iter().map(|entry| (EntryPointType::L1Handler, entry)))
        .fold(HashMap::new(), |mut map, (entry_type, entry)| {
            map.entry(entry_type).or_default().push(from_legacy_entry_point(entry));
            map
        })
}

/// Returns a [LegacyContractEntryPoint] (starknet-rs) from a [EntryPoint]
/// (starknet-api)
fn to_legacy_entry_point(entry_point: EntryPoint) -> Result<LegacyContractEntryPoint, FromByteArrayError> {
    let selector = entry_point.selector.0.into();
    let offset = entry_point.offset.0 as u64;
    Ok(LegacyContractEntryPoint { selector, offset })
}

/// Returns a [SierraEntryPoint] (starknet-rs) from a [EntryPointV1]
/// (starknet-api)
fn to_entry_point(entry_point: EntryPointV1, index: u64) -> Result<SierraEntryPoint, FromByteArrayError> {
    let selector = entry_point.selector.0.into();
    let function_idx = index;
    Ok(SierraEntryPoint { selector, function_idx })
}

/// Returns a [EntryPoint] (starknet-api) from a [LegacyContractEntryPoint]
/// (starknet-rs)
fn from_legacy_entry_point(entry_point: &LegacyContractEntryPoint) -> EntryPoint {
    let selector = EntryPointSelector(StarkFelt::new_unchecked(entry_point.selector.to_bytes_be()));
    let offset = EntryPointOffset(entry_point.offset as usize);
    EntryPoint { selector, offset }
}

use starknet_core::types::{
    FunctionStateMutability, LegacyEventAbiEntry, LegacyEventAbiType, LegacyFunctionAbiEntry, LegacyFunctionAbiType,
    LegacyStructAbiEntry, LegacyStructAbiType, LegacyStructMember, LegacyTypedParameter,
};

// Wrapper Class conversion

impl TryFrom<ContractClassCore> for ContractClassWrapper {
    type Error = anyhow::Error;

    fn try_from(contract_class: ContractClassCore) -> Result<Self, Self::Error> {
        let contract = from_rpc_contract_class(contract_class.clone())?;
        let abi = match &contract_class {
            ContractClassCore::Legacy(class_cairo) => {
                ContractAbi::Cairo(from_rpc_contract_abi(class_cairo.abi.clone()))
            }
            ContractClassCore::Sierra(class_sierra) => ContractAbi::Sierra(class_sierra.abi.clone()),
        };

        let sierra_program_length = match contract_class {
            ContractClassCore::Sierra(class_sierra) => class_sierra.sierra_program.len(),
            ContractClassCore::Legacy(_) => 0,
        } as u64;
        let abi_length = abi.length() as u64;

        Ok(Self { contract, abi, sierra_program_length, abi_length })
    }
}

impl TryInto<ContractClassCore> for ContractClassWrapper {
    type Error = anyhow::Error;

    fn try_into(self) -> Result<ContractClassCore, Self::Error> {
        match self.abi {
            ContractAbi::Cairo(abi_cairo) => {
                if let ContractClassBlockifier::V0(contract) = self.contract {
                    to_contract_class_cairo(&contract, to_rpc_contract_abi(abi_cairo))
                } else {
                    unreachable!("should not mix Cairo abi with Sierra contract")
                }
            }
            ContractAbi::Sierra(abi_sierra) => {
                if let ContractClassBlockifier::V1(contract) = self.contract {
                    to_contract_class_sierra(&contract, abi_sierra)
                } else {
                    unreachable!("should not mix Sierra abi with Cairo contract")
                }
            }
        }
    }
}

fn to_rpc_contract_abi(abi: Option<Vec<AbiEntryWrapper>>) -> Option<Vec<LegacyContractAbiEntry>> {
    abi.map(|entries| entries.into_iter().map(|v| v.into()).collect())
}

fn from_rpc_contract_abi(abi: Option<Vec<LegacyContractAbiEntry>>) -> Option<Vec<AbiEntryWrapper>> {
    abi.map(|entries| entries.into_iter().map(AbiEntryWrapper::from).collect())
}

// TODO: this is in serious need of refactoring

impl From<LegacyContractAbiEntry> for AbiEntryWrapper {
    fn from(abi_entry: LegacyContractAbiEntry) -> Self {
        match abi_entry {
            LegacyContractAbiEntry::Function(abi_function) => {
                AbiEntryWrapper::Function(AbiFunctionEntryWrapper::from(abi_function))
            }
            LegacyContractAbiEntry::Event(abi_event) => AbiEntryWrapper::Event(AbiEventEntryWrapper::from(abi_event)),
            LegacyContractAbiEntry::Struct(abi_struct) => {
                AbiEntryWrapper::Struct(AbiStructEntryWrapper::from(abi_struct))
            }
        }
    }
}

impl From<AbiEntryWrapper> for LegacyContractAbiEntry {
    fn from(abi_entry: AbiEntryWrapper) -> Self {
        match abi_entry {
            AbiEntryWrapper::Function(abi_function) => LegacyContractAbiEntry::Function(abi_function.into()),
            AbiEntryWrapper::Event(abi_event) => LegacyContractAbiEntry::Event(abi_event.into()),
            AbiEntryWrapper::Struct(abi_struct) => LegacyContractAbiEntry::Struct(abi_struct.into()),
        }
    }
}

// Function ABI Entry conversion

impl From<LegacyFunctionAbiEntry> for AbiFunctionEntryWrapper {
    fn from(abi_function_entry: LegacyFunctionAbiEntry) -> Self {
        Self {
            r#type: AbiFunctionTypeWrapper::from(abi_function_entry.r#type),
            name: abi_function_entry.name,
            inputs: abi_function_entry.inputs.into_iter().map(AbiTypedParameterWrapper::from).collect(),
            outputs: abi_function_entry.outputs.into_iter().map(AbiTypedParameterWrapper::from).collect(),
            state_mutability: abi_function_entry.state_mutability.map(AbiFunctionStateMutabilityWrapper::from),
        }
    }
}

impl From<AbiFunctionEntryWrapper> for LegacyFunctionAbiEntry {
    fn from(abi_function_entry: AbiFunctionEntryWrapper) -> Self {
        LegacyFunctionAbiEntry {
            r#type: abi_function_entry.r#type.into(),
            name: abi_function_entry.name,
            inputs: abi_function_entry.inputs.into_iter().map(|v| v.into()).collect(),
            outputs: abi_function_entry.outputs.into_iter().map(|v| v.into()).collect(),
            state_mutability: abi_function_entry.state_mutability.map(|v| v.into()),
        }
    }
}

impl From<LegacyFunctionAbiType> for AbiFunctionTypeWrapper {
    fn from(abi_func_type: LegacyFunctionAbiType) -> Self {
        match abi_func_type {
            LegacyFunctionAbiType::Function => AbiFunctionTypeWrapper::Function,
            LegacyFunctionAbiType::L1Handler => AbiFunctionTypeWrapper::L1handler,
            LegacyFunctionAbiType::Constructor => AbiFunctionTypeWrapper::Constructor,
        }
    }
}

impl From<AbiFunctionTypeWrapper> for LegacyFunctionAbiType {
    fn from(abi_function_type: AbiFunctionTypeWrapper) -> Self {
        match abi_function_type {
            AbiFunctionTypeWrapper::Function => LegacyFunctionAbiType::Function,
            AbiFunctionTypeWrapper::L1handler => LegacyFunctionAbiType::L1Handler,
            AbiFunctionTypeWrapper::Constructor => LegacyFunctionAbiType::Constructor,
        }
    }
}

impl From<FunctionStateMutability> for AbiFunctionStateMutabilityWrapper {
    fn from(abi_func_state_mutability: FunctionStateMutability) -> Self {
        match abi_func_state_mutability {
            FunctionStateMutability::View => AbiFunctionStateMutabilityWrapper::View,
        }
    }
}

impl From<AbiFunctionStateMutabilityWrapper> for FunctionStateMutability {
    fn from(abi_func_state_mutability: AbiFunctionStateMutabilityWrapper) -> Self {
        match abi_func_state_mutability {
            AbiFunctionStateMutabilityWrapper::View => FunctionStateMutability::View,
        }
    }
}

// Event ABI Entry conversion

impl From<LegacyEventAbiEntry> for AbiEventEntryWrapper {
    fn from(abi_event_entry: LegacyEventAbiEntry) -> Self {
        Self {
            r#type: AbiEventTypeWrapper::from(abi_event_entry.r#type),
            name: abi_event_entry.name,
            keys: abi_event_entry.keys.into_iter().map(AbiTypedParameterWrapper::from).collect(),
            data: abi_event_entry.data.into_iter().map(AbiTypedParameterWrapper::from).collect(),
        }
    }
}

impl From<AbiEventEntryWrapper> for LegacyEventAbiEntry {
    fn from(abi_event_entry: AbiEventEntryWrapper) -> Self {
        LegacyEventAbiEntry {
            r#type: abi_event_entry.r#type.into(),
            name: abi_event_entry.name,
            keys: abi_event_entry.keys.into_iter().map(|v| v.into()).collect(),
            data: abi_event_entry.data.into_iter().map(|v| v.into()).collect(),
        }
    }
}

impl From<LegacyEventAbiType> for AbiEventTypeWrapper {
    fn from(abi_entry_type: LegacyEventAbiType) -> Self {
        match abi_entry_type {
            LegacyEventAbiType::Event => AbiEventTypeWrapper::Event,
        }
    }
}

impl From<AbiEventTypeWrapper> for LegacyEventAbiType {
    fn from(abi_event_type: AbiEventTypeWrapper) -> Self {
        match abi_event_type {
            AbiEventTypeWrapper::Event => LegacyEventAbiType::Event,
        }
    }
}

// Struct ABI Entry conversion

impl From<LegacyStructAbiEntry> for AbiStructEntryWrapper {
    fn from(abi_struct_entry: LegacyStructAbiEntry) -> Self {
        Self {
            r#type: AbiStructTypeWrapper::from(abi_struct_entry.r#type),
            name: abi_struct_entry.name,
            size: abi_struct_entry.size,
            members: abi_struct_entry.members.into_iter().map(AbiStructMemberWrapper::from).collect(),
        }
    }
}

impl From<AbiStructEntryWrapper> for LegacyStructAbiEntry {
    fn from(abi_struct_entry: AbiStructEntryWrapper) -> Self {
        LegacyStructAbiEntry {
            r#type: abi_struct_entry.r#type.into(),
            name: abi_struct_entry.name,
            size: abi_struct_entry.size,
            members: abi_struct_entry.members.into_iter().map(LegacyStructMember::from).collect(),
        }
    }
}

impl From<LegacyStructAbiType> for AbiStructTypeWrapper {
    fn from(abi_struct_type: LegacyStructAbiType) -> Self {
        match abi_struct_type {
            LegacyStructAbiType::Struct => AbiStructTypeWrapper::Struct,
        }
    }
}

impl From<AbiStructTypeWrapper> for LegacyStructAbiType {
    fn from(abi_struct_type: AbiStructTypeWrapper) -> Self {
        match abi_struct_type {
            AbiStructTypeWrapper::Struct => LegacyStructAbiType::Struct,
        }
    }
}

impl From<LegacyStructMember> for AbiStructMemberWrapper {
    fn from(abi_struct_member: LegacyStructMember) -> Self {
        Self { name: abi_struct_member.name, r#type: abi_struct_member.r#type, offset: abi_struct_member.offset }
    }
}

impl From<AbiStructMemberWrapper> for LegacyStructMember {
    fn from(abi_struct_member: AbiStructMemberWrapper) -> Self {
        LegacyStructMember {
            name: abi_struct_member.name,
            r#type: abi_struct_member.r#type,
            offset: abi_struct_member.offset,
        }
    }
}

impl From<LegacyTypedParameter> for AbiTypedParameterWrapper {
    fn from(abi_typed_parameter: LegacyTypedParameter) -> Self {
        Self { name: abi_typed_parameter.name, r#type: abi_typed_parameter.r#type }
    }
}

impl From<AbiTypedParameterWrapper> for LegacyTypedParameter {
    fn from(abi_typed_parameter: AbiTypedParameterWrapper) -> Self {
        LegacyTypedParameter { name: abi_typed_parameter.name, r#type: abi_typed_parameter.r#type }
    }
}
