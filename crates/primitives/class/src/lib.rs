use std::ops::Deref;

use starknet_types_core::felt::Felt;

mod class_hash;
mod compile;
mod into_starknet_core;

pub use class_hash::ClassHash;
pub use compile::ToCompiledClass;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ConvertedClass {
    pub class_infos: (Felt, ClassInfo),
    pub class_compiled: (Felt, CompiledClass),
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClassInfo {
    pub contract_class: ContractClass,
    pub compiled_class_hash: Felt,
    /// None means it is in the pending block
    pub block_number: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ContractClass {
    Sierra(FlattenedSierraClass),
    Legacy(CompressedLegacyContractClass),
}

impl ContractClass {
    pub fn sierra_program_length(&self) -> usize {
        match self {
            ContractClass::Sierra(FlattenedSierraClass { sierra_program, .. }) => sierra_program.len(),
            ContractClass::Legacy(_) => 0,
        }
    }

    pub fn abi_length(&self) -> usize {
        match self {
            ContractClass::Sierra(FlattenedSierraClass { abi, .. }) => abi.len(),
            ContractClass::Legacy(_) => 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FlattenedSierraClass {
    pub sierra_program: Vec<Felt>,
    pub contract_class_version: String,
    pub entry_points_by_type: EntryPointsByType,
    pub abi: String,
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
    pub constructor: Vec<LegacyContractEntryPoint>,
    pub external: Vec<LegacyContractEntryPoint>,
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

/// A contract class that has been compiled and can be transformed into a `blockifier::execution::contract_class::ContractClass`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CompiledClass {
    Sierra(CompiledSierra),
    Legacy(CompiledLegacy),
}

impl Deref for CompiledClass {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        match self {
            CompiledClass::Sierra(CompiledSierra(bytes)) => bytes,
            CompiledClass::Legacy(CompiledLegacy(bytes)) => bytes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CompiledSierra(Vec<u8>);

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CompiledLegacy(Vec<u8>);

pub fn to_blockifier_class(
    compiled_class: CompiledClass,
) -> Result<blockifier::execution::contract_class::ContractClass, cairo_vm::types::errors::program_errors::ProgramError>
{
    match compiled_class {
        CompiledClass::Sierra(compiled_class) => Ok(blockifier::execution::contract_class::ContractClass::V1(
            blockifier::execution::contract_class::ContractClassV1::try_from_json_string(
                &String::from_utf8(compiled_class.0).map_err(|err| {
                    cairo_vm::types::errors::program_errors::ProgramError::IO(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        err,
                    ))
                })?,
            )?,
        )),
        CompiledClass::Legacy(compiled_class) => Ok(blockifier::execution::contract_class::ContractClass::V0(
            blockifier::execution::contract_class::ContractClassV0::try_from_json_string(
                &String::from_utf8(compiled_class.0).map_err(|err| {
                    cairo_vm::types::errors::program_errors::ProgramError::IO(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        err,
                    ))
                })?,
            )?,
        )),
    }
}
