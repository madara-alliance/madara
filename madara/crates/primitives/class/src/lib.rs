use class_hash::ComputeClassHashError;
use compile::ClassCompilationError;
use starknet_types_core::felt::Felt;
use std::{collections::HashMap, fmt, sync::Arc};

pub mod class_hash;
pub mod class_update;
pub mod compile;
pub mod convert;
mod into_starknet_core;
mod into_starknet_types;
pub mod mainnet_legacy_class_hashes;
mod to_blockifier;
mod to_starknet_api;

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ConvertedClass {
    Legacy(LegacyConvertedClass),
    Sierra(SierraConvertedClass),
}

impl ConvertedClass {
    pub fn class_hash(&self) -> &Felt {
        match self {
            Self::Legacy(legacy) => &legacy.class_hash,
            Self::Sierra(sierra) => &sierra.class_hash,
        }
    }

    pub fn info(&self) -> ClassInfo {
        match self {
            Self::Legacy(legacy) => ClassInfo::Legacy(legacy.info.clone()),
            Self::Sierra(sierra) => ClassInfo::Sierra(sierra.info.clone()),
        }
    }

    pub fn as_sierra(&self) -> Option<&SierraConvertedClass> {
        match self {
            Self::Sierra(sierra) => Some(sierra),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LegacyConvertedClass {
    pub class_hash: Felt,
    pub info: LegacyClassInfo,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SierraConvertedClass {
    pub class_hash: Felt,
    pub info: SierraClassInfo,
    pub compiled: Arc<CompiledSierra>,
}

#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ClassType {
    Sierra,
    Legacy,
}

impl fmt::Display for ClassType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClassInfoWithHash {
    pub class_info: ClassInfo,
    pub class_hash: Felt,
}

impl ClassInfoWithHash {
    /// Does class compilation. Does check the resulting compiled class hash.
    pub fn convert(self) -> Result<ConvertedClass, ClassCompilationError> {
        match self.class_info {
            ClassInfo::Sierra(sierra_class_info) => {
                let compiled = sierra_class_info.compile()?;
                Ok(ConvertedClass::Sierra(SierraConvertedClass {
                    class_hash: self.class_hash,
                    info: sierra_class_info,
                    compiled: Arc::new(compiled),
                }))
            }
            ClassInfo::Legacy(legacy_class_info) => Ok(ConvertedClass::Legacy(LegacyConvertedClass {
                class_hash: self.class_hash,
                info: legacy_class_info,
            })),
        }
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

    pub fn compiled_class_hash(&self) -> Option<Felt> {
        match self {
            ClassInfo::Sierra(sierra) => Some(sierra.compiled_class_hash),
            ClassInfo::Legacy(_) => None,
        }
    }

    pub fn compute_hash(&self) -> Result<Felt, ComputeClassHashError> {
        match self {
            ClassInfo::Sierra(sierra_class_info) => sierra_class_info.contract_class.compute_class_hash(),
            ClassInfo::Legacy(legacy_class_info) => legacy_class_info.contract_class.compute_class_hash(),
        }
    }

    pub fn with_computed_hash(self) -> Result<ClassInfoWithHash, ComputeClassHashError> {
        Ok(ClassInfoWithHash { class_hash: self.compute_hash()?, class_info: self })
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LegacyClassInfo {
    pub contract_class: Arc<CompressedLegacyContractClass>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SierraClassInfo {
    pub contract_class: Arc<FlattenedSierraClass>,
    pub compiled_class_hash: Felt,
}

impl SierraClassInfo {
    pub fn compile(&self) -> Result<CompiledSierra, ClassCompilationError> {
        let (compiled_class_hash, compiled) = self.contract_class.compile_to_casm()?;
        if self.compiled_class_hash != compiled_class_hash {
            return Err(ClassCompilationError::CompiledClassHashMismatch {
                expected: self.compiled_class_hash,
                got: compiled_class_hash,
            });
        }
        Ok((&compiled).try_into()?)
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

#[derive(Clone, Debug, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

#[derive(Clone, Debug, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CompressedSierraClass {
    /// The gzipped compressed program as a base64 string.
    pub sierra_program: String,
    pub contract_class_version: String,
    pub entry_points_by_type: EntryPointsByType,
    pub abi: String,
}

impl TryFrom<FlattenedSierraClass> for CompressedSierraClass {
    type Error = std::io::Error;

    fn try_from(flattened_sierra_class: FlattenedSierraClass) -> Result<Self, Self::Error> {
        let mut base64_encoder =
            base64::write::EncoderWriter::new(Vec::new(), &base64::engine::general_purpose::STANDARD);
        let mut gzip_encoder = flate2::write::GzEncoder::new(&mut base64_encoder, flate2::Compression::default());
        serde_json::to_writer(&mut gzip_encoder, &flattened_sierra_class.sierra_program)?;
        gzip_encoder.try_finish()?;
        drop(gzip_encoder);
        let encoded_data = base64_encoder
            .finish()
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "base64 encoding error"))?;
        let sierra_program = String::from_utf8(encoded_data)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "base64 encoding error: invalid utf8"))?;

        Ok(Self {
            sierra_program,
            contract_class_version: flattened_sierra_class.contract_class_version,
            entry_points_by_type: flattened_sierra_class.entry_points_by_type,
            abi: flattened_sierra_class.abi,
        })
    }
}

impl TryFrom<CompressedSierraClass> for FlattenedSierraClass {
    type Error = std::io::Error;

    fn try_from(compressed_sierra_class: CompressedSierraClass) -> Result<Self, Self::Error> {
        let s = compressed_sierra_class.sierra_program;
        // base64 -> gz -> json
        let sierra_program = serde_json::from_reader(crate::convert::gz_decompress_stream(
            base64::read::DecoderReader::new(s.as_bytes(), &base64::engine::general_purpose::STANDARD),
        ))?;

        Ok(Self {
            sierra_program,
            contract_class_version: compressed_sierra_class.contract_class_version,
            entry_points_by_type: compressed_sierra_class.entry_points_by_type,
            abi: compressed_sierra_class.abi,
        })
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub struct EntryPointsByType {
    pub constructor: Vec<SierraEntryPoint>,
    pub external: Vec<SierraEntryPoint>,
    pub l1_handler: Vec<SierraEntryPoint>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SierraEntryPoint {
    pub selector: Felt,
    pub function_idx: u64,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CompressedLegacyContractClass {
    pub program: Vec<u8>,
    pub entry_points_by_type: LegacyEntryPointsByType,
    pub abi: Option<Vec<LegacyContractAbiEntry>>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct LegacyContractClass {
    pub entry_points_by_type: starknet_core::types::contract::legacy::RawLegacyEntryPoints,
    pub abi: Option<Vec<starknet_core::types::contract::legacy::RawLegacyAbiEntry>>,
    pub program: starknet_core::types::contract::legacy::LegacyProgram,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LegacyEntryPointsByType {
    #[serde(rename = "CONSTRUCTOR")]
    pub constructor: Vec<LegacyContractEntryPoint>,
    #[serde(rename = "EXTERNAL")]
    pub external: Vec<LegacyContractEntryPoint>,
    #[serde(rename = "L1_HANDLER")]
    pub l1_handler: Vec<LegacyContractEntryPoint>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LegacyContractEntryPoint {
    pub offset: u64,
    pub selector: Felt,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LegacyContractAbiEntry {
    Function(LegacyFunctionAbiEntry),
    Event(LegacyEventAbiEntry),
    Struct(LegacyStructAbiEntry),
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LegacyFunctionAbiEntry {
    pub r#type: LegacyFunctionAbiType,
    pub name: String,
    pub inputs: Vec<LegacyTypedParameter>,
    pub outputs: Vec<LegacyTypedParameter>,
    #[serde(rename = "stateMutability")]
    pub state_mutability: Option<FunctionStateMutability>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LegacyEventAbiEntry {
    pub r#type: LegacyEventAbiType,
    pub name: String,
    pub keys: Vec<LegacyTypedParameter>,
    pub data: Vec<LegacyTypedParameter>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LegacyStructAbiEntry {
    pub r#type: LegacyStructAbiType,
    pub name: String,
    pub size: u64,
    pub members: Vec<LegacyStructMember>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LegacyStructMember {
    pub name: String,
    pub r#type: String,
    pub offset: u64,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LegacyTypedParameter {
    pub name: String,
    pub r#type: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyFunctionAbiType {
    Function,
    L1Handler,
    Constructor,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyEventAbiType {
    Event,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyStructAbiType {
    Struct,
}

#[derive(Clone, Copy, Hash, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FunctionStateMutability {
    View,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CompiledSierra(pub String);

impl AsRef<str> for CompiledSierra {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

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

    #[test]
    fn legacy_class_mainnet_block_20732_no_abi() {
        let class = serde_json::from_str::<LegacyContractClass>(include_str!(
            "../resources/legacy_class_mainnet_block_20732_no_abi.json"
        ))
        .unwrap();

        let real_class_hash =
            Felt::from_hex_unchecked("0x371b5f7c5517d84205365a87f02dcef230efa7b4dd91a9e4ba7e04c5b69d69b");
        let computed_class_hash =
            Felt::from_hex_unchecked("0x92d5e5e82d6eaaef47a8ba076f0ea0989d2c5aeb84d74d8ade33fe773cbf67");
        assert_eq!(class.class_hash().unwrap(), real_class_hash);

        //  TODO to re-check this after 0.14.0 compatibility
        assert_eq!(
            crate::mainnet_legacy_class_hashes::get_real_class_hash(20732, computed_class_hash),
            real_class_hash
        );
    }
}
