use std::{collections::HashMap, sync::Arc};

use starknet_types_core::felt::Felt;

pub mod class_hash;
pub mod class_update;
pub mod compile;
pub mod convert;
mod into_starknet_core;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConvertedClass {
    Legacy(LegacyConvertedClass),
    Sierra(SierraConvertedClass),
}

impl ConvertedClass {
    pub fn class_hash(&self) -> Felt {
        match self {
            ConvertedClass::Legacy(legacy) => legacy.class_hash,
            ConvertedClass::Sierra(sierra) => sierra.class_hash,
        }
    }

    pub fn info(&self) -> ClassInfo {
        match self {
            ConvertedClass::Legacy(legacy) => ClassInfo::Legacy(legacy.info.clone()),
            ConvertedClass::Sierra(sierra) => ClassInfo::Sierra(sierra.info.clone()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LegacyConvertedClass {
    pub class_hash: Felt,
    pub info: LegacyClassInfo,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SierraConvertedClass {
    pub class_hash: Felt,
    pub info: SierraClassInfo,
    pub compiled: Arc<CompiledSierra>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ClassInfo {
    Sierra(SierraClassInfo),
    Legacy(LegacyClassInfo),
}

impl From<LegacyClassInfo> for ClassInfo {
    fn from(legacy_class_info: LegacyClassInfo) -> Self {
        ClassInfo::Legacy(legacy_class_info)
    }
}

impl From<SierraClassInfo> for ClassInfo {
    fn from(sierra_class_info: SierraClassInfo) -> Self {
        ClassInfo::Sierra(sierra_class_info)
    }
}

impl ClassInfo {
    pub fn contract_class(&self) -> ContractClass {
        match self {
            ClassInfo::Sierra(sierra) => ContractClass::Sierra(Arc::clone(&sierra.contract_class)),
            ClassInfo::Legacy(legacy) => ContractClass::Legacy(Arc::clone(&legacy.contract_class)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LegacyClassInfo {
    pub contract_class: Arc<CompressedLegacyContractClass>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SierraClassInfo {
    pub contract_class: Arc<FlattenedSierraClass>,
    pub compiled_class_hash: Felt,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ContractClass {
    Sierra(Arc<FlattenedSierraClass>),
    Legacy(Arc<CompressedLegacyContractClass>),
}

impl From<FlattenedSierraClass> for ContractClass {
    fn from(flattened_sierra_class: FlattenedSierraClass) -> Self {
        ContractClass::Sierra(Arc::new(flattened_sierra_class))
    }
}

impl From<CompressedLegacyContractClass> for ContractClass {
    fn from(compressed_legacy_contract_class: CompressedLegacyContractClass) -> Self {
        ContractClass::Legacy(Arc::new(compressed_legacy_contract_class))
    }
}

impl ContractClass {
    pub fn sierra_program_length(&self) -> usize {
        match self {
            ContractClass::Sierra(sierra) => sierra.program_length(),
            ContractClass::Legacy(_) => 0,
        }
    }

    pub fn abi_length(&self) -> usize {
        match self {
            ContractClass::Sierra(sierra) => sierra.abi_length(),
            ContractClass::Legacy(_) => 0,
        }
    }

    pub fn is_sierra(&self) -> bool {
        matches!(self, ContractClass::Sierra(_))
    }

    pub fn is_legacy(&self) -> bool {
        matches!(self, ContractClass::Legacy(_))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FlattenedSierraClass {
    pub sierra_program: Vec<Felt>,
    pub contract_class_version: String,
    pub entry_points_by_type: EntryPointsByType,
    pub abi: String,
}

impl FlattenedSierraClass {
    pub fn program_length(&self) -> usize {
        self.sierra_program.len()
    }

    pub fn abi_length(&self) -> usize {
        self.abi.len()
    }
}

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

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LegacyEntryPointsByType {
    #[serde(rename = "CONSTRUCTOR")]
    pub constructor: Vec<LegacyContractEntryPoint>,
    #[serde(rename = "EXTERNAL")]
    pub external: Vec<LegacyContractEntryPoint>,
    #[serde(rename = "L1_HANDLER")]
    pub l1_handler: Vec<LegacyContractEntryPoint>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LegacyContractEntryPoint {
    pub offset: u64,
    pub selector: Felt,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LegacyContractAbiEntry {
    Function(LegacyFunctionAbiEntry),
    Event(LegacyEventAbiEntry),
    Struct(LegacyStructAbiEntry),
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LegacyFunctionAbiEntry {
    pub r#type: LegacyFunctionAbiType,
    pub name: String,
    pub inputs: Vec<LegacyTypedParameter>,
    pub outputs: Vec<LegacyTypedParameter>,
    pub state_mutability: Option<FunctionStateMutability>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LegacyEventAbiEntry {
    pub r#type: LegacyEventAbiType,
    pub name: String,
    pub keys: Vec<LegacyTypedParameter>,
    pub data: Vec<LegacyTypedParameter>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LegacyStructAbiEntry {
    pub r#type: LegacyStructAbiType,
    pub name: String,
    pub size: u64,
    pub members: Vec<LegacyStructMember>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LegacyStructMember {
    pub name: String,
    pub r#type: String,
    pub offset: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LegacyTypedParameter {
    pub name: String,
    pub r#type: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LegacyFunctionAbiType {
    Function,
    L1Handler,
    Constructor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LegacyEventAbiType {
    Event,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LegacyStructAbiType {
    Struct,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FunctionStateMutability {
    View,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CompiledSierra(String);

const MISSED_CLASS_HASHES_JSON: &[u8] = include_bytes!("../resources/missed_classes.json");

lazy_static::lazy_static! {
    pub static ref MISSED_CLASS_HASHES: HashMap::<u64, Vec<Felt>> =
        serde_json::from_slice(MISSED_CLASS_HASHES_JSON).unwrap();
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_load_missing_class_hashes() {
        let missed_class_hashes = &MISSED_CLASS_HASHES;
        assert_eq!(missed_class_hashes.len(), 38);
        assert_eq!(missed_class_hashes.iter().map(|(_, v)| v.len()).sum::<usize>(), 57);
    }
}
