use std::io::Read;
use std::sync::Arc;

use anyhow::Context;
use blockifier::execution::contract_class::{
    self, ContractClass as ContractClassBlockifier, ContractClassV0, ContractClassV0Inner, ContractClassV1
};
use cairo_vm::types::program::Program;
use dp_convert::ToStarkFelt;
use dp_transactions::from_broadcasted_transactions::flattened_sierra_to_casm_contract_class;
use flate2::read::GzDecoder;
use indexmap::IndexMap;
use parity_scale_codec::{Decode, Encode};
use starknet_api::core::{ClassHash, EntryPointSelector, Nonce};
use starknet_api::deprecated_contract_class::{EntryPoint, EntryPointOffset, EntryPointType};
use starknet_core::types::contract::legacy::{LegacyContractClass, RawLegacyEntryPoint, RawLegacyEntryPoints};
use starknet_core::types::contract::{ComputeClassHashError, JsonError};
use starknet_core::types::{
    CompressedLegacyContractClass, ContractClass as ContractClassCore, Felt, FlattenedSierraClass, LegacyContractAbiEntry, SierraEntryPoint
};
use starknet_providers::sequencer::models::DeployedClass;
use starknet_types_core::hash::{Poseidon, StarkHash};
use starknet_core::utils::{cairo_short_string_to_felt, normalize_address, starknet_keccak};

/// Cairo string for "CONTRACT_CLASS_V0.1.0"
const PREFIX_CONTRACT_CLASS_V0_1_0: Felt = Felt::from_raw([
    37302452645455172,
    18446734822722598327,
    15539482671244488427,
    5800711240972404213,
]);

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
    let cairo_class: LegacyContractClass = serde_json::from_str(json_str)?;
    let compressed_cairo_class = cairo_class.compress().expect("Cairo program compression failed");
    // if some abi serialize it to Option<Vec<LegacyContractAbiEntry, Global>>
    let abi = abi.map(|abi| {
        let abi_entries: Vec<LegacyContractAbiEntry> = serde_json::from_str(&abi).expect("Failed to deserialize abi");
        abi_entries
    });
    Ok(ContractClassCore::Legacy(CompressedLegacyContractClass {
        program: compressed_cairo_class.program,
        abi,
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

/// Returns a [EntryPoint] (starknet-api) from a [RawLegacyEntryPoint]
/// (starknet-rs)
fn from_legacy_entry_point(entry_point: &RawLegacyEntryPoint) -> EntryPoint {
    let selector = EntryPointSelector(entry_point.selector.to_stark_felt());
    let offset = EntryPointOffset(entry_point.offset.into());
    EntryPoint { selector, offset }
}

fn is_sierra(class: &serde_json::Value) -> bool {
    class.get("sierra_program").is_some()
}

// Wrapper Class conversion
impl TryFrom<(serde_json::Value, Felt)> for ContractClassWrapper {
    type Error = anyhow::Error;

    fn try_from(value: (serde_json::Value, Felt)) -> Result<Self, Self::Error> {
        let contract_class = value.0;
        let class_hash = value.1;
        
        // Computing hashes to verify the fetched class
        let computed_class_hash = if is_sierra(&contract_class) {
            compute_sierra_class_hash(contract_class.clone())?
        } else {
            compute_cairo_class_hash(contract_class.clone())?
        };
        
        if computed_class_hash != class_hash {
            return Err(anyhow::anyhow!("❗ Computed class hash does not match the provided class hash"));
        }
        
        let sierra_program = contract_class.get("sierra_program");
        let sierra_program_length = match sierra_program {
            Some(serde_json::Value::Array(program)) => program.len() as u64,
            _ => 0, // Set length to 0 if sierra_program is missing or not an array
        };
        
        let abi_value = contract_class.get("abi");
        let abi = if is_sierra(&contract_class) {
            match abi_value {
                Some(abi_value) => {
                    let abi_string =
                    abi_value.as_str().ok_or_else(|| anyhow::anyhow!("Invalid `abi` field format"))?.to_string();
                    ContractAbi::Sierra(abi_string)
                }
                None => return Err(anyhow::anyhow!("Missing `abi` field for Sierra program")),
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
        
        let class = contract_class.to_string();

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

pub fn compute_cairo_class_hash(raw_class: serde_json::Value) -> Result<Felt, ComputeClassHashError> {
    let mut elements = Vec::new();
    let class: LegacyContractClass = serde_json::from_value(raw_class).expect("Failed to convert contract class");

    // API Version
    elements.push(Felt::ZERO);

    // Hashes external entry points
    elements.push({
        let mut buffer = Vec::new();
        for entrypoint in class.entry_points_by_type.external.iter() {
            buffer.push(entrypoint.selector);
            buffer.push(entrypoint.offset.into());
        }
        Poseidon::hash_array(&buffer)
    });

    // Hashes L1 handler entry points
    elements.push({
        let mut buffer = Vec::new();
        for entrypoint in class.entry_points_by_type.l1_handler.iter() {
            buffer.push(entrypoint.selector);
            buffer.push(entrypoint.offset.into());
        }
        Poseidon::hash_array(&buffer)
    });

    // Hashes constructor entry points
    elements.push({
        let mut buffer = Vec::new();
        for entrypoint in class.entry_points_by_type.constructor.iter() {
            buffer.push(entrypoint.selector);
            buffer.push(entrypoint.offset.into());
        }
        Poseidon::hash_array(&buffer)
    });

    // Hashes builtins
    elements.push(Poseidon::hash_array(
        &class
            .program
            .builtins
            .iter()
            .map(|item| cairo_short_string_to_felt(item))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ComputeClassHashError::InvalidBuiltinName)?,
    ));

    // Hashes hinted_class_hash
    elements.push(class.hinted_class_hash()?);

    // Hashes bytecode
    elements.push(Poseidon::hash_array(&class.program.data));

    Ok(Poseidon::hash_array(&elements))
}

pub fn compute_sierra_class_hash(raw_class: serde_json::Value) -> Result<Felt, ComputeClassHashError> {
    let mut elements = Vec::new();
    let class: FlattenedSierraClass = serde_json::from_value(raw_class).expect("Failed to convert contract class");
    elements.push(PREFIX_CONTRACT_CLASS_V0_1_0);

    // Hashes entry points
    elements.push({
        let mut buffer = Vec::new();
        for entrypoint in class.entry_points_by_type.external.iter() {
            buffer.push(entrypoint.selector);
            buffer.push(entrypoint.function_idx.into());
        }
        Poseidon::hash_array(&buffer)
    });
    elements.push({
        let mut buffer = Vec::new();
        for entrypoint in class.entry_points_by_type.constructor.iter() {
            buffer.push(entrypoint.selector);
            buffer.push(entrypoint.function_idx.into());
        }
        Poseidon::hash_array(&buffer)
    });
    elements.push({
        let mut buffer = Vec::new();
        for entrypoint in class.entry_points_by_type.l1_handler.iter() {
            buffer.push(entrypoint.selector);
            buffer.push(entrypoint.function_idx.into());
        }
        Poseidon::hash_array(&buffer)
    });

    // Hashes ABI
    elements.push(starknet_keccak(class.abi.as_bytes()));

    // Hashes Sierra program
    elements.push(Poseidon::hash_array(&class.sierra_program));

    Ok(normalize_address(Poseidon::hash_array(&elements)))
}

