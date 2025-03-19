use crate::{CompiledSierra, CompressedLegacyContractClass, FlattenedSierraClass, LegacyContractAbiEntry};
use casm_classes_v2::casm_contract_class::CasmContractClass;
use num_bigint::{BigInt, BigUint, Sign};
use starknet_types_core::felt::Felt;
use std::borrow::Cow;

#[cfg(feature = "cairo_native")]
use cairo_native::executor::AotContractExecutor;

#[derive(Debug, thiserror::Error)]
pub enum ClassCompilationError {
    #[error("Failed to decompress program: {0}")]
    DecompressionFailed(#[from] std::io::Error),
    #[error("Failed to parse program JSON: {0}")]
    ParsingProgramJsonFailed(#[from] serde_json::Error),
    #[error("Program is not an object")]
    ProgramIsNotAnObject,
    #[error("Failed to compile siera class: {0}")]
    CompilationFailed(String), // use String due to different crates versions for compilation
    #[error("Failed to parse sierra version: {0}")]
    ParsingSierraVersion(Cow<'static, str>),
    #[error("Failed to construct a blockifier class: {0}")]
    BlockifierClassConstructionFailed(#[from] cairo_vm::types::errors::program_errors::ProgramError),
    #[error("Compiled class hash mismatch, expected {expected:#x} got {got:#x}")]
    CompiledClassHashMismatch { expected: Felt, got: Felt },
}

impl CompressedLegacyContractClass {
    // Returns `impl serde::Serialize` because the fact that it returns a serde_json::Value is an impl detail
    pub fn abi(&self) -> Result<impl serde::Serialize, ClassCompilationError> {
        // This convoluted JSON serialization is a way to get around bincode's
        // lack of support for #[serde(tag = "type")]. Abi entries should be
        // serialized as typed JSON structs, so we have to do this manually.
        //
        // NOTE: that the `type` field is already present in each ABI entry
        // struct so we do not need to add it manually.

        // we should actually change that, it would be better to have a concrete type here.

        let abi = self
            .abi
            .as_ref()
            .map(|abi| {
                abi.iter()
                    .map(|entry| match entry {
                        LegacyContractAbiEntry::Function(entry) => serde_json::to_value(entry).map(|mut v| {
                            if entry.state_mutability.is_none() {
                                v.as_object_mut().unwrap().remove("stateMutability");
                            }
                            v
                        }),
                        LegacyContractAbiEntry::Event(entry) => serde_json::to_value(entry),
                        LegacyContractAbiEntry::Struct(entry) => serde_json::to_value(entry),
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(ClassCompilationError::ParsingProgramJsonFailed)
            })
            .transpose()?;

        Ok(abi)
    }

    pub fn serialize_to_json(&self) -> Result<String, ClassCompilationError> {
        let mut program: serde_json::Value =
            serde_json::from_reader(crate::convert::gz_decompress_stream(self.program.as_slice()))?;

        let program_object = program.as_object_mut().ok_or(ClassCompilationError::ProgramIsNotAnObject)?;

        if !program_object.contains_key("debug_info") {
            program_object.insert("debug_info".to_owned(), serde_json::json!(""));
        }

        let json = serde_json::json!({
            "program": program,
            "entry_points_by_type": self.entry_points_by_type,
            "abi": self.abi()?,
        });

        Ok(serde_json::to_string(&json)?)
    }

    pub fn to_starknet_api_no_abi(
        &self,
    ) -> Result<starknet_api::deprecated_contract_class::ContractClass, ClassCompilationError> {
        let decoder = flate2::read::GzDecoder::new(std::io::Cursor::new(&self.program));
        let program: starknet_api::deprecated_contract_class::Program = serde_json::from_reader(decoder)?;

        Ok(starknet_api::deprecated_contract_class::ContractClass {
            program,
            entry_points_by_type: self.entry_points_by_type.clone().into(),
            abi: None,
        })
    }
}

impl FlattenedSierraClass {
    /// compiles a [FlattenedSierraClass] to a CASM contract Class
    ///
    /// # Returns
    ///
    /// A tuple containing the compiled class hash and the compiled class serialized to JSON
    pub fn compile_to_casm(&self) -> Result<(Felt, CasmContractClass), ClassCompilationError> {
        let sierra_version = parse_sierra_version(&self.sierra_program)?;

        let (compiled_class_hash, compiled_class) = match sierra_version {
            SierraVersion(0, 1, 0) => {
                let compiled_class = v1_0_0_alpha6::compile(self)?;
                let json = serde_json::to_string(&compiled_class)?;
                let compiled_class: CasmContractClass = serde_json::from_str(&json)?;
                let compiled_class_hash = compiled_class.compiled_class_hash();
                (compiled_class_hash, compiled_class)
            }
            SierraVersion(1, 0, 0) => {
                let compiled_class = v1_0_0_rc0::compile(self)?;
                let json = serde_json::to_string(&compiled_class)?;
                let compiled_class: CasmContractClass = serde_json::from_str(&json)?;
                let compiled_class_hash = compiled_class.compiled_class_hash();
                (compiled_class_hash, compiled_class)
            }
            SierraVersion(1, 1, 0) => {
                let compiled_class = v1_1_1::compile(self)?;
                let json = serde_json::to_string(&compiled_class)?;
                let compiled_class: CasmContractClass = serde_json::from_str(&json)?;
                let compiled_class_hash = compiled_class.compiled_class_hash();
                (compiled_class_hash, compiled_class)
            }
            _ => v2::compile(self)?,
        };
        Ok((compiled_class_hash, compiled_class))
    }

    #[cfg(feature = "cairo_native")]
    pub fn compile_to_native(&self, path: &std::path::Path) -> Result<AotContractExecutor, ClassCompilationError> {
        let sierra_version = parse_sierra_version(&self.sierra_program)?;
        let sierra_version = casm_classes_v2::compiler_version::VersionId {
            major: sierra_version.0 as _,
            minor: sierra_version.1 as _,
            patch: sierra_version.2 as _,
        };
        let sierra = v2::to_cairo_lang(self);
        let program = sierra.extract_sierra_program().unwrap();

        let executor = AotContractExecutor::new_into(
            &program,
            &sierra.entry_points_by_type,
            sierra_version,
            path,
            cairo_native::OptLevel::Default,
        )
        .unwrap()
        .unwrap();

        Ok(executor)
    }

    pub fn sierra_version(&self) -> Result<starknet_api::contract_class::SierraVersion, ClassCompilationError> {
        let version = parse_sierra_version(&self.sierra_program)?;
        Ok(starknet_api::contract_class::SierraVersion::new(version.0, version.1, version.2))
    }
}

impl CompiledSierra {
    pub fn to_casm(&self) -> Result<CasmContractClass, ClassCompilationError> {
        Ok(serde_json::from_str(&self.0)?)
    }
}

impl TryFrom<&CasmContractClass> for CompiledSierra {
    type Error = ClassCompilationError;

    fn try_from(value: &CasmContractClass) -> Result<Self, Self::Error> {
        Ok(CompiledSierra(serde_json::to_string::<CasmContractClass>(value)?))
    }
}

#[derive(Debug, PartialEq)]
struct SierraVersion(u64, u64, u64);

fn parse_sierra_version(program: &[Felt]) -> Result<SierraVersion, ClassCompilationError> {
    const VERSION_0_1_0_AS_SHORTSTRING: Felt = Felt::from_hex_unchecked("0x302e312e30"); // "0.1.0"

    match program {
        [first, ..] if first == &VERSION_0_1_0_AS_SHORTSTRING => Ok(SierraVersion(0, 1, 0)),
        [a, b, c, ..] => {
            let (a, b, c) = (
                (*a).try_into().map_err(|_| ClassCompilationError::ParsingSierraVersion("malformed version".into()))?,
                (*b).try_into().map_err(|_| ClassCompilationError::ParsingSierraVersion("malformed version".into()))?,
                (*c).try_into().map_err(|_| ClassCompilationError::ParsingSierraVersion("malformed version".into()))?,
            );
            Ok(SierraVersion(a, b, c))
        }
        _ => Err(ClassCompilationError::ParsingSierraVersion("Program is too short".into())),
    }
}

mod v1_0_0_alpha6 {
    use crate::{EntryPointsByType, FlattenedSierraClass, SierraEntryPoint};

    use super::{felt_to_big_uint, ClassCompilationError, Felt};
    use casm_compiler_v1_0_0_alpha6::casm_contract_class::CasmContractClass;
    use casm_compiler_v1_0_0_alpha6::contract_class::{ContractClass, ContractEntryPoint, ContractEntryPoints};
    use casm_utils_v1_0_0_alpha6::bigint::BigUintAsHex;

    pub(super) fn compile(sierra: &FlattenedSierraClass) -> Result<CasmContractClass, ClassCompilationError> {
        let sierra_class = to_cairo_lang(sierra);

        let casm_class = CasmContractClass::from_contract_class(sierra_class, true)
            .map_err(|e| ClassCompilationError::CompilationFailed(e.to_string()))?;

        Ok(casm_class)
    }

    fn to_cairo_lang(class: &FlattenedSierraClass) -> ContractClass {
        ContractClass {
            sierra_program: class.sierra_program.iter().map(felt_to_big_uint_as_hex).collect(),
            sierra_program_debug_info: None,
            contract_class_version: class.contract_class_version.clone(),
            entry_points_by_type: entry_points_by_type_to_contract_entry_points(&class.entry_points_by_type),
            abi: None,
        }
    }

    /// Converts a [EntryPointsByType] to a [ContractEntryPoints]
    fn entry_points_by_type_to_contract_entry_points(value: &EntryPointsByType) -> ContractEntryPoints {
        fn sierra_entry_point_to_contract_entry_point(value: SierraEntryPoint) -> ContractEntryPoint {
            ContractEntryPoint {
                function_idx: value.function_idx.try_into().unwrap(),
                selector: felt_to_big_uint(&value.selector),
            }
        }
        ContractEntryPoints {
            constructor: value
                .constructor
                .iter()
                .map(|x| sierra_entry_point_to_contract_entry_point(x.clone()))
                .collect(),
            external: value.external.iter().map(|x| sierra_entry_point_to_contract_entry_point(x.clone())).collect(),
            l1_handler: value
                .l1_handler
                .iter()
                .map(|x| sierra_entry_point_to_contract_entry_point(x.clone()))
                .collect(),
        }
    }

    /// Converts a [Felt] to a [BigUintAsHex]
    fn felt_to_big_uint_as_hex(value: &Felt) -> BigUintAsHex {
        BigUintAsHex { value: felt_to_big_uint(value) }
    }
}

mod v1_0_0_rc0 {
    use crate::{EntryPointsByType, FlattenedSierraClass, SierraEntryPoint};

    use super::{felt_to_big_uint, ClassCompilationError, Felt};
    use casm_compiler_v1_0_0_rc0::casm_contract_class::CasmContractClass;
    use casm_compiler_v1_0_0_rc0::contract_class::{ContractClass, ContractEntryPoint, ContractEntryPoints};
    use casm_utils_v1_0_0_rc0::bigint::BigUintAsHex;

    pub(super) fn compile(sierra: &FlattenedSierraClass) -> Result<CasmContractClass, ClassCompilationError> {
        let sierra_class = to_cairo_lang(sierra);

        let casm_class = CasmContractClass::from_contract_class(sierra_class, true)
            .map_err(|e| ClassCompilationError::CompilationFailed(e.to_string()))?;

        Ok(casm_class)
    }

    fn to_cairo_lang(class: &FlattenedSierraClass) -> ContractClass {
        ContractClass {
            sierra_program: class.sierra_program.iter().map(felt_to_big_uint_as_hex).collect(),
            sierra_program_debug_info: None,
            contract_class_version: class.contract_class_version.clone(),
            entry_points_by_type: entry_points_by_type_to_contract_entry_points(&class.entry_points_by_type),
            abi: None,
        }
    }

    /// Converts a [EntryPointsByType] to a [ContractEntryPoints]
    fn entry_points_by_type_to_contract_entry_points(value: &EntryPointsByType) -> ContractEntryPoints {
        fn sierra_entry_point_to_contract_entry_point(value: SierraEntryPoint) -> ContractEntryPoint {
            ContractEntryPoint {
                function_idx: value.function_idx.try_into().unwrap(),
                selector: felt_to_big_uint(&value.selector),
            }
        }
        ContractEntryPoints {
            constructor: value
                .constructor
                .iter()
                .map(|x| sierra_entry_point_to_contract_entry_point(x.clone()))
                .collect(),
            external: value.external.iter().map(|x| sierra_entry_point_to_contract_entry_point(x.clone())).collect(),
            l1_handler: value
                .l1_handler
                .iter()
                .map(|x| sierra_entry_point_to_contract_entry_point(x.clone()))
                .collect(),
        }
    }

    /// Converts a [Felt] to a [BigUintAsHex]
    fn felt_to_big_uint_as_hex(value: &Felt) -> BigUintAsHex {
        BigUintAsHex { value: felt_to_big_uint(value) }
    }
}

mod v1_1_1 {
    use crate::{EntryPointsByType, FlattenedSierraClass, SierraEntryPoint};

    use super::{felt_to_big_uint, ClassCompilationError, Felt};
    use casm_compiler_v1_1_1::casm_contract_class::CasmContractClass;
    use casm_compiler_v1_1_1::contract_class::{ContractClass, ContractEntryPoint, ContractEntryPoints};
    use casm_utils_v1_1_1::bigint::BigUintAsHex;

    pub(super) fn compile(sierra: &FlattenedSierraClass) -> Result<CasmContractClass, ClassCompilationError> {
        let sierra_class = to_cairo_lang(sierra);

        let casm_class = CasmContractClass::from_contract_class(sierra_class, true)
            .map_err(|e| ClassCompilationError::CompilationFailed(e.to_string()))?;

        Ok(casm_class)
    }

    fn to_cairo_lang(class: &FlattenedSierraClass) -> ContractClass {
        ContractClass {
            sierra_program: class.sierra_program.iter().map(felt_to_big_uint_as_hex).collect(),
            sierra_program_debug_info: None,
            contract_class_version: class.contract_class_version.clone(),
            entry_points_by_type: entry_points_by_type_to_contract_entry_points(&class.entry_points_by_type),
            abi: None,
        }
    }

    /// Converts a [EntryPointsByType] to a [ContractEntryPoints]
    fn entry_points_by_type_to_contract_entry_points(value: &EntryPointsByType) -> ContractEntryPoints {
        fn sierra_entry_point_to_contract_entry_point(value: SierraEntryPoint) -> ContractEntryPoint {
            ContractEntryPoint {
                function_idx: value.function_idx.try_into().unwrap(),
                selector: felt_to_big_uint(&value.selector),
            }
        }
        ContractEntryPoints {
            constructor: value
                .constructor
                .iter()
                .map(|x| sierra_entry_point_to_contract_entry_point(x.clone()))
                .collect(),
            external: value.external.iter().map(|x| sierra_entry_point_to_contract_entry_point(x.clone())).collect(),
            l1_handler: value
                .l1_handler
                .iter()
                .map(|x| sierra_entry_point_to_contract_entry_point(x.clone()))
                .collect(),
        }
    }

    /// Converts a [Felt] to a [BigUintAsHex]
    fn felt_to_big_uint_as_hex(value: &Felt) -> BigUintAsHex {
        BigUintAsHex { value: felt_to_big_uint(value) }
    }
}

mod v2 {
    use crate::{EntryPointsByType, FlattenedSierraClass, SierraEntryPoint};

    use super::{felt_to_big_uint, ClassCompilationError, Felt};
    use casm_classes_v2::casm_contract_class::CasmContractClass;
    use casm_classes_v2::contract_class::{ContractClass, ContractEntryPoint, ContractEntryPoints};
    use casm_utils_v2::bigint::BigUintAsHex;

    pub(super) fn compile(sierra: &FlattenedSierraClass) -> Result<(Felt, CasmContractClass), ClassCompilationError> {
        let sierra_class = to_cairo_lang(sierra);

        let casm_class = CasmContractClass::from_contract_class(sierra_class, true, usize::MAX)
            .map_err(|e| ClassCompilationError::CompilationFailed(e.to_string()))?;
        let compiled_class_hash = casm_class.compiled_class_hash();

        Ok((compiled_class_hash, casm_class))
    }

    pub(super) fn to_cairo_lang(class: &FlattenedSierraClass) -> ContractClass {
        ContractClass {
            sierra_program: class.sierra_program.iter().map(felt_to_big_uint_as_hex).collect(),
            sierra_program_debug_info: None,
            contract_class_version: class.contract_class_version.clone(),
            entry_points_by_type: entry_points_by_type_to_contract_entry_points(&class.entry_points_by_type),
            abi: None,
        }
    }

    /// Converts a [EntryPointsByType] to a [ContractEntryPoints]
    fn entry_points_by_type_to_contract_entry_points(value: &EntryPointsByType) -> ContractEntryPoints {
        fn sierra_entry_point_to_contract_entry_point(value: SierraEntryPoint) -> ContractEntryPoint {
            ContractEntryPoint {
                function_idx: value.function_idx.try_into().unwrap(),
                selector: felt_to_big_uint(&value.selector),
            }
        }
        ContractEntryPoints {
            constructor: value
                .constructor
                .iter()
                .map(|x| sierra_entry_point_to_contract_entry_point(x.clone()))
                .collect(),
            external: value.external.iter().map(|x| sierra_entry_point_to_contract_entry_point(x.clone())).collect(),
            l1_handler: value
                .l1_handler
                .iter()
                .map(|x| sierra_entry_point_to_contract_entry_point(x.clone()))
                .collect(),
        }
    }

    /// Converts a [Felt] to a [BigUintAsHex]
    fn felt_to_big_uint_as_hex(value: &Felt) -> BigUintAsHex {
        BigUintAsHex { value: felt_to_big_uint(value) }
    }
}

/// Converts a [Felt] to a [BigUint]
fn felt_to_big_uint(value: &Felt) -> BigUint {
    BigInt::from_bytes_be(Sign::Plus, &value.to_bytes_be()).to_biguint().unwrap()
}

#[cfg(test)]
mod tests {
    use crate::ContractClass;
    use starknet_core::types::BlockId;
    use starknet_core::types::BlockTag;
    use starknet_providers::{Provider, SequencerGatewayProvider};
    use starknet_types_core::felt::Felt;

    #[tokio::test]
    async fn test_compressed_legacy_class_to_blockifier() {
        let provider = SequencerGatewayProvider::starknet_alpha_mainnet();
        let class_hash = Felt::from_hex_unchecked("0x25ec026985a3bf9d0cc1fe17326b245dfdc3ff89b8fde106542a3ea56c5a918");
        let _class: ContractClass =
            provider.get_class(BlockId::Tag(BlockTag::Latest), class_hash).await.unwrap().into();
    }

    #[tokio::test]
    async fn test_flattened_sierra_class_to_blockifier() {
        let provider = SequencerGatewayProvider::starknet_alpha_mainnet();

        let class_hash = Felt::from_hex_unchecked("0x816dd0297efc55dc1e7559020a3a825e81ef734b558f03c83325d4da7e6253");
        let expected_compiled_class_hash =
            Felt::from_hex_unchecked("0x3b3c33049515b020e435d5803b2b8a6915398cbcd8f79ca545a04c286c5084b");
        let class: ContractClass = provider.get_class(BlockId::Tag(BlockTag::Latest), class_hash).await.unwrap().into();

        if let ContractClass::Sierra(sierra) = class {
            let start = std::time::Instant::now();
            let (compiled_class_hash, _casm_definition) = sierra.compile_to_casm().unwrap();
            println!("compile time: {:?}", start.elapsed());
            assert_eq!(compiled_class_hash, expected_compiled_class_hash);
        } else {
            panic!("Not a Sierra contract");
        }
    }
}
