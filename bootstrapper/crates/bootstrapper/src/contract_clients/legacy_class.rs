// Note: This code has been taken from the madara code-base.

use base64;
use starknet_types_core::felt::Felt;

pub type Address = Felt;
pub type Signature = Vec<Felt>;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EntryPointsByType {
    pub constructor: Vec<SierraEntryPoint>,
    pub external: Vec<SierraEntryPoint>,
    pub l1_handler: Vec<SierraEntryPoint>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SierraEntryPoint {
    pub selector: Felt,
    pub function_idx: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CompressedLegacyContractClass {
    pub program: Vec<u8>,
    pub entry_points_by_type: LegacyEntryPointsByType,
    pub abi: Option<Vec<LegacyContractAbiEntry>>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DeprecatedContractClass {
    #[serde(default)]
    pub abi: Option<Vec<LegacyContractAbiEntry>>,
    pub entry_points_by_type: LegacyEntryPointsByType,
    /// A base64 representation of the compressed program code
    pub program: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct LegacyEntryPointsByType {
    #[serde(rename = "CONSTRUCTOR")]
    pub constructor: Vec<LegacyContractEntryPoint>,
    #[serde(rename = "EXTERNAL")]
    pub external: Vec<LegacyContractEntryPoint>,
    #[serde(rename = "L1_HANDLER")]
    pub l1_handler: Vec<LegacyContractEntryPoint>,
}

// First, we need to define the NumAsHex serialization helper
mod num_as_hex {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::fmt::Write;

    pub fn serialize<S, T>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: Copy + Into<u64>,
    {
        let mut hex = String::with_capacity(18);
        write!(&mut hex, "0x{:x}", (*value).into()).unwrap();
        serializer.serialize_str(&hex)
    }

    pub fn deserialize<'de, D, T>(deserializer: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: TryFrom<u64>,
        <T as TryFrom<u64>>::Error: std::fmt::Display,
    {
        let s = String::deserialize(deserializer)?;
        let without_prefix = s.strip_prefix("0x").unwrap_or(&s);

        let parsed = u64::from_str_radix(without_prefix, 16)
            .map_err(|e| serde::de::Error::custom(format!("Failed to parse hex: {}", e)))?;

        T::try_from(parsed).map_err(|e| serde::de::Error::custom(e.to_string()))
    }
}

// Add this attribute to allow the non-snake_case warning for NumAsHex
#[allow(non_snake_case)]
pub mod NumAsHex {
    pub use super::num_as_hex::{deserialize, serialize};
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct LegacyContractEntryPoint {
    #[serde(with = "NumAsHex")]
    pub offset: u64,
    pub selector: Felt,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum LegacyContractAbiEntry {
    Function(LegacyFunctionAbiEntry),
    Event(LegacyEventAbiEntry),
    Struct(LegacyStructAbiEntry),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct LegacyFunctionAbiEntry {
    pub r#type: LegacyFunctionAbiType,
    pub name: String,
    pub inputs: Vec<LegacyTypedParameter>,
    pub outputs: Vec<LegacyTypedParameter>,
    #[serde(rename = "stateMutability")]
    pub state_mutability: Option<FunctionStateMutability>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct LegacyEventAbiEntry {
    pub r#type: LegacyEventAbiType,
    pub name: String,
    pub keys: Vec<LegacyTypedParameter>,
    pub data: Vec<LegacyTypedParameter>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct LegacyStructAbiEntry {
    pub r#type: LegacyStructAbiType,
    pub name: String,
    pub size: u64,
    pub members: Vec<LegacyStructMember>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct LegacyStructMember {
    pub name: String,
    pub r#type: String,
    pub offset: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct LegacyTypedParameter {
    pub name: String,
    pub r#type: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum LegacyFunctionAbiType {
    #[serde(rename = "function")]
    Function,
    #[serde(rename = "l1_handler")]
    L1Handler,
    #[serde(rename = "constructor")]
    Constructor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum LegacyEventAbiType {
    #[serde(rename = "event")]
    Event,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum LegacyStructAbiType {
    #[serde(rename = "struct")]
    Struct,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum FunctionStateMutability {
    #[serde(rename = "view")]
    View,
}

impl From<starknet_core::types::EntryPointsByType> for EntryPointsByType {
    fn from(entry_points_by_type: starknet_core::types::EntryPointsByType) -> Self {
        EntryPointsByType {
            constructor: entry_points_by_type
                .constructor
                .into_iter()
                .map(|sierra_entry_point| sierra_entry_point.into())
                .collect(),
            external: entry_points_by_type
                .external
                .into_iter()
                .map(|sierra_entry_point| sierra_entry_point.into())
                .collect(),
            l1_handler: entry_points_by_type
                .l1_handler
                .into_iter()
                .map(|sierra_entry_point| sierra_entry_point.into())
                .collect(),
        }
    }
}

impl From<EntryPointsByType> for starknet_core::types::EntryPointsByType {
    fn from(entry_points_by_type: EntryPointsByType) -> Self {
        starknet_core::types::EntryPointsByType {
            constructor: entry_points_by_type
                .constructor
                .into_iter()
                .map(|sierra_entry_point| sierra_entry_point.into())
                .collect(),
            external: entry_points_by_type
                .external
                .into_iter()
                .map(|sierra_entry_point| sierra_entry_point.into())
                .collect(),
            l1_handler: entry_points_by_type
                .l1_handler
                .into_iter()
                .map(|sierra_entry_point| sierra_entry_point.into())
                .collect(),
        }
    }
}

impl From<starknet_core::types::SierraEntryPoint> for SierraEntryPoint {
    fn from(sierra_entry_point: starknet_core::types::SierraEntryPoint) -> Self {
        SierraEntryPoint { selector: sierra_entry_point.selector, function_idx: sierra_entry_point.function_idx }
    }
}

impl From<SierraEntryPoint> for starknet_core::types::SierraEntryPoint {
    fn from(sierra_entry_point: SierraEntryPoint) -> Self {
        starknet_core::types::SierraEntryPoint {
            selector: sierra_entry_point.selector,
            function_idx: sierra_entry_point.function_idx,
        }
    }
}

impl From<starknet_core::types::CompressedLegacyContractClass> for CompressedLegacyContractClass {
    fn from(compressed_legacy_contract_class: starknet_core::types::CompressedLegacyContractClass) -> Self {
        CompressedLegacyContractClass {
            program: compressed_legacy_contract_class.program,
            entry_points_by_type: compressed_legacy_contract_class.entry_points_by_type.into(),
            abi: compressed_legacy_contract_class
                .abi
                .map(|abi| abi.into_iter().map(|legacy_contract_abi_entry| legacy_contract_abi_entry.into()).collect()),
        }
    }
}

impl From<CompressedLegacyContractClass> for starknet_core::types::CompressedLegacyContractClass {
    fn from(compressed_legacy_contract_class: CompressedLegacyContractClass) -> Self {
        starknet_core::types::CompressedLegacyContractClass {
            program: compressed_legacy_contract_class.program,
            entry_points_by_type: compressed_legacy_contract_class.entry_points_by_type.into(),
            abi: compressed_legacy_contract_class
                .abi
                .map(|abi| abi.into_iter().map(|legacy_contract_abi_entry| legacy_contract_abi_entry.into()).collect()),
        }
    }
}

impl From<starknet_core::types::LegacyEntryPointsByType> for LegacyEntryPointsByType {
    fn from(legacy_entry_points_by_type: starknet_core::types::LegacyEntryPointsByType) -> Self {
        LegacyEntryPointsByType {
            constructor: legacy_entry_points_by_type
                .constructor
                .into_iter()
                .map(|legacy_contract_entry_point| legacy_contract_entry_point.into())
                .collect(),
            external: legacy_entry_points_by_type
                .external
                .into_iter()
                .map(|legacy_contract_entry_point| legacy_contract_entry_point.into())
                .collect(),
            l1_handler: legacy_entry_points_by_type
                .l1_handler
                .into_iter()
                .map(|legacy_contract_entry_point| legacy_contract_entry_point.into())
                .collect(),
        }
    }
}

impl From<LegacyEntryPointsByType> for starknet_core::types::LegacyEntryPointsByType {
    fn from(legacy_entry_points_by_type: LegacyEntryPointsByType) -> Self {
        starknet_core::types::LegacyEntryPointsByType {
            constructor: legacy_entry_points_by_type
                .constructor
                .into_iter()
                .map(|legacy_contract_entry_point| legacy_contract_entry_point.into())
                .collect(),
            external: legacy_entry_points_by_type
                .external
                .into_iter()
                .map(|legacy_contract_entry_point| legacy_contract_entry_point.into())
                .collect(),
            l1_handler: legacy_entry_points_by_type
                .l1_handler
                .into_iter()
                .map(|legacy_contract_entry_point| legacy_contract_entry_point.into())
                .collect(),
        }
    }
}

impl From<starknet_core::types::LegacyContractEntryPoint> for LegacyContractEntryPoint {
    fn from(legacy_contract_entry_point: starknet_core::types::LegacyContractEntryPoint) -> Self {
        LegacyContractEntryPoint {
            offset: legacy_contract_entry_point.offset,
            selector: legacy_contract_entry_point.selector,
        }
    }
}

impl From<LegacyContractEntryPoint> for starknet_core::types::LegacyContractEntryPoint {
    fn from(legacy_contract_entry_point: LegacyContractEntryPoint) -> Self {
        starknet_core::types::LegacyContractEntryPoint {
            offset: legacy_contract_entry_point.offset,
            selector: legacy_contract_entry_point.selector,
        }
    }
}

impl From<starknet_core::types::LegacyContractAbiEntry> for LegacyContractAbiEntry {
    fn from(legacy_contract_abi_entry: starknet_core::types::LegacyContractAbiEntry) -> Self {
        match legacy_contract_abi_entry {
            starknet_core::types::LegacyContractAbiEntry::Function(legacy_function_abi_entry) => {
                LegacyContractAbiEntry::Function(legacy_function_abi_entry.into())
            }
            starknet_core::types::LegacyContractAbiEntry::Event(legacy_event_abi_entry) => {
                LegacyContractAbiEntry::Event(legacy_event_abi_entry.into())
            }
            starknet_core::types::LegacyContractAbiEntry::Struct(legacy_struct_abi_entry) => {
                LegacyContractAbiEntry::Struct(legacy_struct_abi_entry.into())
            }
        }
    }
}

impl From<LegacyContractAbiEntry> for starknet_core::types::LegacyContractAbiEntry {
    fn from(legacy_contract_abi_entry: LegacyContractAbiEntry) -> Self {
        match legacy_contract_abi_entry {
            LegacyContractAbiEntry::Function(legacy_function_abi_entry) => {
                starknet_core::types::LegacyContractAbiEntry::Function(legacy_function_abi_entry.into())
            }
            LegacyContractAbiEntry::Event(legacy_event_abi_entry) => {
                starknet_core::types::LegacyContractAbiEntry::Event(legacy_event_abi_entry.into())
            }
            LegacyContractAbiEntry::Struct(legacy_struct_abi_entry) => {
                starknet_core::types::LegacyContractAbiEntry::Struct(legacy_struct_abi_entry.into())
            }
        }
    }
}

impl From<starknet_core::types::LegacyFunctionAbiEntry> for LegacyFunctionAbiEntry {
    fn from(legacy_function_abi_entry: starknet_core::types::LegacyFunctionAbiEntry) -> Self {
        LegacyFunctionAbiEntry {
            r#type: legacy_function_abi_entry.r#type.into(),
            name: legacy_function_abi_entry.name,
            inputs: legacy_function_abi_entry.inputs.into_iter().map(|abi_entry| abi_entry.into()).collect(),
            outputs: legacy_function_abi_entry.outputs.into_iter().map(|abi_entry| abi_entry.into()).collect(),
            state_mutability: legacy_function_abi_entry
                .state_mutability
                .map(|state_mutability| state_mutability.into()),
        }
    }
}

impl From<LegacyFunctionAbiEntry> for starknet_core::types::LegacyFunctionAbiEntry {
    fn from(legacy_function_abi_entry: LegacyFunctionAbiEntry) -> Self {
        starknet_core::types::LegacyFunctionAbiEntry {
            r#type: legacy_function_abi_entry.r#type.into(),
            name: legacy_function_abi_entry.name,
            inputs: legacy_function_abi_entry.inputs.into_iter().map(|abi_entry| abi_entry.into()).collect(),
            outputs: legacy_function_abi_entry.outputs.into_iter().map(|abi_entry| abi_entry.into()).collect(),
            state_mutability: legacy_function_abi_entry
                .state_mutability
                .map(|state_mutability| state_mutability.into()),
        }
    }
}

impl From<starknet_core::types::LegacyEventAbiEntry> for LegacyEventAbiEntry {
    fn from(legacy_event_abi_entry: starknet_core::types::LegacyEventAbiEntry) -> Self {
        LegacyEventAbiEntry {
            r#type: legacy_event_abi_entry.r#type.into(),
            name: legacy_event_abi_entry.name,
            keys: legacy_event_abi_entry.keys.into_iter().map(|abi_entry| abi_entry.into()).collect(),
            data: legacy_event_abi_entry.data.into_iter().map(|abi_entry| abi_entry.into()).collect(),
        }
    }
}

impl From<LegacyEventAbiEntry> for starknet_core::types::LegacyEventAbiEntry {
    fn from(legacy_event_abi_entry: LegacyEventAbiEntry) -> Self {
        starknet_core::types::LegacyEventAbiEntry {
            r#type: legacy_event_abi_entry.r#type.into(),
            name: legacy_event_abi_entry.name,
            keys: legacy_event_abi_entry.keys.into_iter().map(|abi_entry| abi_entry.into()).collect(),
            data: legacy_event_abi_entry.data.into_iter().map(|abi_entry| abi_entry.into()).collect(),
        }
    }
}

impl From<starknet_core::types::LegacyStructAbiEntry> for LegacyStructAbiEntry {
    fn from(legacy_struct_abi_entry: starknet_core::types::LegacyStructAbiEntry) -> Self {
        LegacyStructAbiEntry {
            r#type: legacy_struct_abi_entry.r#type.into(),
            name: legacy_struct_abi_entry.name,
            size: legacy_struct_abi_entry.size,
            members: legacy_struct_abi_entry.members.into_iter().map(|member| member.into()).collect(),
        }
    }
}

impl From<LegacyStructAbiEntry> for starknet_core::types::LegacyStructAbiEntry {
    fn from(legacy_struct_abi_entry: LegacyStructAbiEntry) -> Self {
        starknet_core::types::LegacyStructAbiEntry {
            r#type: legacy_struct_abi_entry.r#type.into(),
            name: legacy_struct_abi_entry.name,
            size: legacy_struct_abi_entry.size,
            members: legacy_struct_abi_entry.members.into_iter().map(|member| member.into()).collect(),
        }
    }
}

impl From<starknet_core::types::LegacyStructMember> for LegacyStructMember {
    fn from(legacy_struct_member: starknet_core::types::LegacyStructMember) -> Self {
        LegacyStructMember {
            name: legacy_struct_member.name,
            r#type: legacy_struct_member.r#type,
            offset: legacy_struct_member.offset,
        }
    }
}

impl From<LegacyStructMember> for starknet_core::types::LegacyStructMember {
    fn from(legacy_struct_member: LegacyStructMember) -> Self {
        starknet_core::types::LegacyStructMember {
            name: legacy_struct_member.name,
            r#type: legacy_struct_member.r#type,
            offset: legacy_struct_member.offset,
        }
    }
}

impl From<starknet_core::types::LegacyTypedParameter> for LegacyTypedParameter {
    fn from(legacy_typed_parameter: starknet_core::types::LegacyTypedParameter) -> Self {
        LegacyTypedParameter { r#type: legacy_typed_parameter.r#type, name: legacy_typed_parameter.name }
    }
}

impl From<LegacyTypedParameter> for starknet_core::types::LegacyTypedParameter {
    fn from(legacy_typed_parameter: LegacyTypedParameter) -> Self {
        starknet_core::types::LegacyTypedParameter {
            r#type: legacy_typed_parameter.r#type,
            name: legacy_typed_parameter.name,
        }
    }
}

impl From<starknet_core::types::LegacyFunctionAbiType> for LegacyFunctionAbiType {
    fn from(legacy_function_abi_type: starknet_core::types::LegacyFunctionAbiType) -> Self {
        match legacy_function_abi_type {
            starknet_core::types::LegacyFunctionAbiType::Function => LegacyFunctionAbiType::Function,
            starknet_core::types::LegacyFunctionAbiType::L1Handler => LegacyFunctionAbiType::L1Handler,
            starknet_core::types::LegacyFunctionAbiType::Constructor => LegacyFunctionAbiType::Constructor,
        }
    }
}

impl From<LegacyFunctionAbiType> for starknet_core::types::LegacyFunctionAbiType {
    fn from(legacy_function_abi_type: LegacyFunctionAbiType) -> Self {
        match legacy_function_abi_type {
            LegacyFunctionAbiType::Function => starknet_core::types::LegacyFunctionAbiType::Function,
            LegacyFunctionAbiType::L1Handler => starknet_core::types::LegacyFunctionAbiType::L1Handler,
            LegacyFunctionAbiType::Constructor => starknet_core::types::LegacyFunctionAbiType::Constructor,
        }
    }
}

impl From<starknet_core::types::LegacyEventAbiType> for LegacyEventAbiType {
    fn from(legacy_event_abi_type: starknet_core::types::LegacyEventAbiType) -> Self {
        match legacy_event_abi_type {
            starknet_core::types::LegacyEventAbiType::Event => LegacyEventAbiType::Event,
        }
    }
}

impl From<LegacyEventAbiType> for starknet_core::types::LegacyEventAbiType {
    fn from(legacy_event_abi_type: LegacyEventAbiType) -> Self {
        match legacy_event_abi_type {
            LegacyEventAbiType::Event => starknet_core::types::LegacyEventAbiType::Event,
        }
    }
}

impl From<starknet_core::types::LegacyStructAbiType> for LegacyStructAbiType {
    fn from(legacy_struct_abi_type: starknet_core::types::LegacyStructAbiType) -> Self {
        match legacy_struct_abi_type {
            starknet_core::types::LegacyStructAbiType::Struct => LegacyStructAbiType::Struct,
        }
    }
}

impl From<LegacyStructAbiType> for starknet_core::types::LegacyStructAbiType {
    fn from(legacy_struct_abi_type: LegacyStructAbiType) -> Self {
        match legacy_struct_abi_type {
            LegacyStructAbiType::Struct => starknet_core::types::LegacyStructAbiType::Struct,
        }
    }
}

impl From<starknet_core::types::FunctionStateMutability> for FunctionStateMutability {
    fn from(function_state_mutability: starknet_core::types::FunctionStateMutability) -> Self {
        match function_state_mutability {
            starknet_core::types::FunctionStateMutability::View => FunctionStateMutability::View,
        }
    }
}

impl From<FunctionStateMutability> for starknet_core::types::FunctionStateMutability {
    fn from(function_state_mutability: FunctionStateMutability) -> Self {
        match function_state_mutability {
            FunctionStateMutability::View => starknet_core::types::FunctionStateMutability::View,
        }
    }
}

impl From<starknet_core::types::CompressedLegacyContractClass> for DeprecatedContractClass {
    fn from(compressed_legacy_contract_class: starknet_core::types::CompressedLegacyContractClass) -> Self {
        // Add this attribute to allow the deprecated function warning for base64::encode
        #[allow(deprecated)]
        let program_base64 = base64::encode(&compressed_legacy_contract_class.program);

        DeprecatedContractClass {
            program: program_base64,
            entry_points_by_type: compressed_legacy_contract_class.entry_points_by_type.into(),
            abi: compressed_legacy_contract_class
                .abi
                .map(|abi| abi.into_iter().map(|legacy_contract_abi_entry| legacy_contract_abi_entry.into()).collect()),
        }
    }
}

impl From<DeprecatedContractClass> for starknet_core::types::CompressedLegacyContractClass {
    fn from(deprecated_contract_class: DeprecatedContractClass) -> Self {
        // Add this attribute to allow the deprecated function warning for base64::decode
        #[allow(deprecated)]
        let program_bytes =
            base64::decode(&deprecated_contract_class.program).expect("Failed to decode base64 program");

        starknet_core::types::CompressedLegacyContractClass {
            program: program_bytes,
            entry_points_by_type: deprecated_contract_class.entry_points_by_type.into(),
            abi: deprecated_contract_class
                .abi
                .map(|abi| abi.into_iter().map(|legacy_contract_abi_entry| legacy_contract_abi_entry.into()).collect()),
        }
    }
}
