//! Contract class compilation utilities
//!
//! This module provides functions for compiling Starknet contract classes from various
//! formats (legacy, Sierra) to executable formats (CASM, Cairo Native).
//!
//! # Compilation Targets
//!
//! - **CASM**: Cairo Assembly format used by Cairo VM
//! - **Cairo Native**: Ahead-of-time compiled native code (`.so` files) for faster execution
//!
//! # Key Functions
//!
//! - `compile_to_casm()`: Compiles Sierra to CASM for VM execution
//! - `compile_to_native()`: Compiles Sierra to native code for Cairo Native execution
//!
//! # Error Handling
//!
//! All compilation functions return `Result<T, ClassCompilationError>` with detailed
//! error messages for debugging compilation failures.

use crate::{CompiledSierra, CompressedLegacyContractClass, FlattenedSierraClass, LegacyContractAbiEntry};
use casm_classes_v2::casm_contract_class::CasmContractClass;
use num_bigint::{BigInt, BigUint, Sign};
use starknet_types_core::felt::Felt;

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
    ParsingSierraVersion(#[from] SierraVersionError),
    #[error("Failed to construct a blockifier class: {0}")]
    BlockifierClassConstructionFailed(#[from] cairo_vm::types::errors::program_errors::ProgramError),
    #[error("Compiled class hash mismatch, expected {expected:#x} got {got:#x}")]
    CompiledClassHashMismatch { expected: Felt, got: Felt },
    #[error("Failed to compile sierra to cairo native: {0}")]
    NativeCompilationFailed(cairo_native::error::Error),
    #[error("Failed to extract sierra program")]
    ExtractSierraProgramFailed(String), // use String due to original error type Felt252SerdeError not being available publicly
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
    ) -> Result<starknet_api::deprecated_contract_class::ContractClass, serde_json::Error> {
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

    /// Compiles a [FlattenedSierraClass] to CASM and computes both Poseidon and BLAKE2s hashes.
    ///
    /// # Returns
    ///
    /// A tuple containing:
    /// - Poseidon compiled class hash (pre-SNIP-34)
    /// - BLAKE2s compiled class hash (post-SNIP-34)
    /// - The compiled CASM class
    pub fn compile_to_casm_with_blake_hash(&self) -> Result<(Felt, Felt, CasmContractClass), ClassCompilationError> {
        let (poseidon_hash, casm_class) = self.compile_to_casm()?;
        let blake_hash = v2::compute_blake_compiled_class_hash(&casm_class)?;
        Ok((poseidon_hash, blake_hash, casm_class))
    }

    /// Compile this Sierra class to Cairo Native format.
    ///
    /// This function performs ahead-of-time compilation of the Sierra program to native
    /// code, writing the compiled binary to the specified path. The resulting executor
    /// can be used for native execution instead of the Cairo VM.
    ///
    /// # Arguments
    ///
    /// * `path` - File system path where the compiled native binary (`.so` file) will be stored.
    ///   The file must not be locked by another process, or compilation will fail.
    ///
    /// # Returns
    ///
    /// Returns an `AotContractExecutor` that can execute the contract natively, or a
    /// `ClassCompilationError` if compilation fails.
    ///
    /// # Errors
    ///
    /// - `NativeCompilationFailed`: Cairo Native compilation failed
    /// - `ExtractSierraProgramFailed`: Failed to extract Sierra program from class
    /// - `ParsingSierraVersion`: Failed to parse Sierra version
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use std::path::PathBuf;
    /// # use mp_class::CompiledSierra;
    ///
    /// # let sierra_class: CompiledSierra = todo!();
    /// let path = PathBuf::from("/tmp/contract.so");
    /// let executor = sierra_class.compile_to_native(&path)?;
    /// ```
    pub fn compile_to_native(&self, path: &std::path::Path) -> Result<AotContractExecutor, ClassCompilationError> {
        let sierra_version = parse_sierra_version(&self.sierra_program)?;
        let sierra_version = casm_classes_v2::compiler_version::VersionId {
            major: sierra_version.0 as _,
            minor: sierra_version.1 as _,
            patch: sierra_version.2 as _,
        };
        let sierra = v2::to_cairo_lang(self);
        let program = sierra.extract_sierra_program().map_err(|e| {
            ClassCompilationError::ExtractSierraProgramFailed(format!("Failed to extract Sierra program: {}", e))
        })?;

        let executor = AotContractExecutor::new_into(
            &program,
            &sierra.entry_points_by_type,
            sierra_version,
            path,
            cairo_native::OptLevel::Default,
            None, // Statistics
        )
        .map_err(ClassCompilationError::NativeCompilationFailed)?
        .ok_or(ClassCompilationError::ExtractSierraProgramFailed(
            "Cairo native compilation returned None (file lock held by another process)".into(),
        ))?;

        Ok(executor)
    }

    pub fn sierra_version(&self) -> Result<starknet_api::contract_class::SierraVersion, SierraVersionError> {
        let version = parse_sierra_version(&self.sierra_program)?;
        Ok(starknet_api::contract_class::SierraVersion::new(version.0, version.1, version.2))
    }
}

impl TryFrom<&CompiledSierra> for CasmContractClass {
    type Error = serde_json::Error;

    fn try_from(value: &CompiledSierra) -> Result<Self, Self::Error> {
        serde_json::from_str(&value.0)
    }
}

impl TryFrom<&CasmContractClass> for CompiledSierra {
    type Error = serde_json::Error;

    fn try_from(value: &CasmContractClass) -> Result<Self, Self::Error> {
        serde_json::to_string::<CasmContractClass>(value).map(CompiledSierra)
    }
}

#[derive(Debug, PartialEq)]
struct SierraVersion(u64, u64, u64);

#[derive(Debug, thiserror::Error)]
pub enum SierraVersionError {
    #[error("Malformed version")]
    MalformedVersion,
    #[error("Program is too short, expected at least 3 elements")]
    TooShortProgram,
}

fn parse_sierra_version(program: &[Felt]) -> Result<SierraVersion, SierraVersionError> {
    const VERSION_0_1_0_AS_SHORTSTRING: Felt = Felt::from_hex_unchecked("0x302e312e30"); // "0.1.0"

    match program {
        [first, ..] if first == &VERSION_0_1_0_AS_SHORTSTRING => Ok(SierraVersion(0, 1, 0)),
        [a, b, c, ..] => {
            let (a, b, c) = (
                (*a).try_into().map_err(|_| SierraVersionError::MalformedVersion)?,
                (*b).try_into().map_err(|_| SierraVersionError::MalformedVersion)?,
                (*c).try_into().map_err(|_| SierraVersionError::MalformedVersion)?,
            );
            Ok(SierraVersion(a, b, c))
        }
        _ => Err(SierraVersionError::TooShortProgram),
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

        let casm_class = CasmContractClass::from_contract_class(
            sierra_class,
            /* add_pythonic_hints */ true,
            /* max_bytecode_size */ usize::MAX,
        )
        .map_err(|e| ClassCompilationError::CompilationFailed(e.to_string()))?;
        let compiled_class_hash = casm_class.compiled_class_hash();

        Ok((compiled_class_hash, casm_class))
    }

    /// Computes the BLAKE2s compiled class hash (SNIP-34) for a CASM contract class.
    ///
    /// Uses starknet_api's HashVersion::V2 for BLAKE2s hashing.
    pub(super) fn compute_blake_compiled_class_hash(
        casm_class: &CasmContractClass,
    ) -> Result<Felt, ClassCompilationError> {
        use starknet_api::contract_class::compiled_class_hash::{HashVersion, HashableCompiledClass};

        let casm_hash_v2 = casm_class.hash(&HashVersion::V2);

        Ok(casm_hash_v2.0)
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
    use starknet_providers::Url;
    use starknet_providers::{Provider, SequencerGatewayProvider};
    use starknet_types_core::felt::Felt;

    #[tokio::test]
    async fn test_compressed_legacy_class_to_blockifier() {
        let provider = SequencerGatewayProvider::new(
            Url::parse("https://gateway.alpha-mainnet.starknet.io/gateway").unwrap(),
            Url::parse("https://feeder.alpha-mainnet.starknet.io/feeder_gateway").unwrap(),
            starknet_core::chain_id::MAINNET,
        );
        let class_hash = Felt::from_hex_unchecked("0x25ec026985a3bf9d0cc1fe17326b245dfdc3ff89b8fde106542a3ea56c5a918");
        let _class: ContractClass =
            provider.get_class(BlockId::Tag(BlockTag::Latest), class_hash).await.unwrap().into();
    }

    #[tokio::test]
    async fn test_flattened_sierra_class_to_blockifier() {
        let provider = SequencerGatewayProvider::new(
            Url::parse("https://gateway.alpha-mainnet.starknet.io/gateway").unwrap(),
            Url::parse("https://feeder.alpha-mainnet.starknet.io/feeder_gateway").unwrap(),
            starknet_core::chain_id::MAINNET,
        );

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

    /// Test that BLAKE compiled class hash (SNIP-34) matches real migrated values from Sepolia.
    ///
    /// These are real migrated_compiled_classes from the first Starknet v0.14.1 block on Sepolia.
    /// Block: https://sepolia.voyager.online/block/0x926ef481b290f748064bc2e4eac931fd3a736645d7daa7a731297c315bcf04
    ///
    /// The class_hash is the Sierra class hash, and compiled_class_hash is the new BLAKE hash.
    #[tokio::test]
    async fn test_blake_compiled_class_hash() {
        // Use Sepolia testnet gateway
        let provider = SequencerGatewayProvider::new(
            Url::parse("https://gateway.alpha-sepolia.starknet.io/gateway").unwrap(),
            Url::parse("https://feeder.alpha-sepolia.starknet.io/feeder_gateway").unwrap(),
            starknet_core::chain_id::SEPOLIA,
        );

        // Real migrated classes from Sepolia after SNIP-34 activation
        // Format: (class_hash, expected_blake_compiled_class_hash)
        let test_cases = [
            (
                "0x345354e2d801833068de73d1a2028e2f619f71045dd5229e79469fa7f598038",
                "0x4ba630be0cd6cdb8d4407bf7c4715b0780e63f890b26b032e8eccaf9c7338e2",
            ),
            (
                "0x2f6d77cb0bca422706a91858dff62975aef4b8214520aadb1f0b39c51f5fde",
                "0x1fa60b7af141c3514c8c193b76dec4a14e54a2474e0d85a053fe61b431edaf3",
            ),
            (
                "0xe824b9f2aa225812cf230d276784b99f182ec95066d84be90cd1682e4ad069",
                "0x292e005a8cd53053cdf4667d74e96da648a5fdac00bfb2616a2051631cc62b6",
            ),
            (
                "0x9524a94b41c4440a16fd96d7c1ef6ad6f44c1c013e96662734502cd4ee9b1f",
                "0x75a59a0cd1985dd66a3fd9d634f0e0287916cd8e0fff3f2bd80d69498b09367",
            ),
            (
                "0x76791ef97c042f81fbf352ad95f39a22554ee8d7927b2ce3c681f3418b5206a",
                "0x538d6278bead12c526309ac2c4e33666008f36fe2195f4ae828a61d350586a",
            ),
            (
                "0x5431265f9d2416426da800a23ddd3fe33db8e2b9fe96dbc48588ac3ac70c091",
                "0x42a545401aaa2b157a445cef59070d47e05e68b84ab669f2dd1fcea307780c6",
            ),
            (
                "0x36078334509b514626504edc9fb252328d1a240e4e948bef8d0c08dff45927f",
                "0x294a323246c017d00a98f11942e1e38d562c97bc79426742110a14ce497e9b5",
            ),
        ];

        // Test the first class (we test just one to keep the test fast, but all are available)
        let (class_hash_hex, expected_blake_hash_hex) = test_cases[0];
        let class_hash = Felt::from_hex_unchecked(class_hash_hex);
        let expected_blake_hash = Felt::from_hex_unchecked(expected_blake_hash_hex);

        let class: ContractClass = provider
            .get_class(BlockId::Tag(BlockTag::Latest), class_hash)
            .await
            .expect("Failed to fetch class from Sepolia")
            .into();

        if let ContractClass::Sierra(sierra) = class {
            // Compile to CASM and get Poseidon hash
            let (poseidon_hash, casm_class) = sierra.compile_to_casm().unwrap();

            // Compute BLAKE hash using HashVersion::V2
            let blake_hash = super::v2::compute_blake_compiled_class_hash(&casm_class).unwrap();

            // Verify BLAKE hash matches expected value from Sepolia migration
            assert_eq!(
                blake_hash, expected_blake_hash,
                "BLAKE hash mismatch! Expected: {:#x}, Got: {:#x}",
                expected_blake_hash, blake_hash
            );

            // BLAKE and Poseidon hashes should be different
            assert_ne!(poseidon_hash, blake_hash, "BLAKE and Poseidon hashes should be different");
        } else {
            panic!("Not a Sierra contract");
        }
    }
}
