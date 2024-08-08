use std::io::{Cursor, Read};

use anyhow::Context;
use dp_convert::felt_to_u64;
use num_bigint::{BigInt, BigUint, Sign};
use starknet_types_core::felt::Felt;

use crate::{CompiledClass, CompiledLegacy, CompiledSierra};

pub trait ToCompiledClass {
    fn compile(&self) -> anyhow::Result<CompiledClass>;
}

impl ToCompiledClass for starknet_core::types::ContractClass {
    fn compile(&self) -> anyhow::Result<CompiledClass> {
        match self {
            starknet_core::types::ContractClass::Sierra(class) => class.compile(),
            starknet_core::types::ContractClass::Legacy(class) => class.compile(),
        }
    }
}

impl ToCompiledClass for starknet_core::types::CompressedLegacyContractClass {
    fn compile(&self) -> anyhow::Result<CompiledClass> {
        let mut decompressor = flate2::read::GzDecoder::new(Cursor::new(&self.program));
        let mut program = Vec::new();
        decompressor.read_to_end(&mut program).context("Decompressing program")?;

        let mut program: serde_json::Value = serde_json::from_slice(&program).context("Parsing program JSON")?;

        let program_object = program.as_object_mut().context("Program attribute was not an object")?;

        if !program_object.contains_key("debug_info") {
            program_object.insert("debug_info".to_owned(), serde_json::json!(""));
        }

        let json = serde_json::json!({
            "program": program,
            "entry_points_by_type": self.entry_points_by_type,
            "abi": self.abi
        });

        let serialized = serde_json::to_vec(&json)?;

        Ok(CompiledClass::Legacy(CompiledLegacy(serialized)))
    }
}

impl ToCompiledClass for starknet_core::types::FlattenedSierraClass {
    fn compile(&self) -> anyhow::Result<CompiledClass> {
        Ok(CompiledClass::Sierra(CompiledSierra(compile_to_casm(self)?)))
    }
}

pub fn compile_to_casm(sierra: &starknet_core::types::FlattenedSierraClass) -> anyhow::Result<Vec<u8>> {
    let sierra_version = parse_sierra_version(&sierra.sierra_program).context("Parsing Sierra version")?;

    match sierra_version {
        SierraVersion(0, 1, 0) => v1_0_0_alpha6::compile(sierra),
        SierraVersion(1, 0, 0) => v1_0_0_rc0::compile(sierra),
        SierraVersion(1, 1, 0) => v1_1_1::compile(sierra),
        _ => v2::compile(sierra),
    }
}

#[derive(Debug, PartialEq)]
struct SierraVersion(u64, u64, u64);

fn parse_sierra_version(program: &[Felt]) -> anyhow::Result<SierraVersion> {
    const VERSION_0_1_0_AS_SHORTSTRING: Felt = Felt::from_hex_unchecked("0x302e312e30");

    match program {
        [first, ..] if first == &VERSION_0_1_0_AS_SHORTSTRING => Ok(SierraVersion(0, 1, 0)),
        [a, b, c, ..] => {
            let (a, b, c) = (felt_to_u64(a)?, felt_to_u64(b)?, felt_to_u64(c)?);
            Ok(SierraVersion(a, b, c))
        }
        _ => anyhow::bail!("Invalid Sierra version"),
    }
}

mod v1_0_0_alpha6 {
    use super::{felt_to_big_uint, Context, Felt};
    use casm_compiler_v1_0_0_alpha6::casm_contract_class::CasmContractClass;
    use casm_compiler_v1_0_0_alpha6::contract_class::{ContractClass, ContractEntryPoint, ContractEntryPoints};
    use casm_utils_v1_0_0_alpha6::bigint::BigUintAsHex;

    pub(super) fn compile(sierra: &starknet_core::types::FlattenedSierraClass) -> anyhow::Result<Vec<u8>> {
        let sierra_class = to_cairo_lang(sierra);

        let casm_class = CasmContractClass::from_contract_class(sierra_class, true).context("Compiling to CASM")?;
        let casm_definition = serde_json::to_vec(&casm_class)?;

        Ok(casm_definition)
    }

    fn to_cairo_lang(class: &starknet_core::types::FlattenedSierraClass) -> ContractClass {
        ContractClass {
            sierra_program: class.sierra_program.iter().map(felt_to_big_uint_as_hex).collect(),
            sierra_program_debug_info: None,
            contract_class_version: class.contract_class_version.clone(),
            entry_points_by_type: entry_points_by_type_to_contract_entry_points(&class.entry_points_by_type),
            abi: None,
        }
    }

    /// Converts a [EntryPointsByType] to a [ContractEntryPoints]
    fn entry_points_by_type_to_contract_entry_points(
        value: &starknet_core::types::EntryPointsByType,
    ) -> ContractEntryPoints {
        fn sierra_entry_point_to_contract_entry_point(
            value: starknet_core::types::SierraEntryPoint,
        ) -> ContractEntryPoint {
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
    use super::{felt_to_big_uint, Context, Felt};
    use casm_compiler_v1_0_0_rc0::casm_contract_class::CasmContractClass;
    use casm_compiler_v1_0_0_rc0::contract_class::{ContractClass, ContractEntryPoint, ContractEntryPoints};
    use casm_utils_v1_0_0_rc0::bigint::BigUintAsHex;

    pub(super) fn compile(sierra: &starknet_core::types::FlattenedSierraClass) -> anyhow::Result<Vec<u8>> {
        let sierra_class = to_cairo_lang(sierra);

        let casm_class = CasmContractClass::from_contract_class(sierra_class, true).context("Compiling to CASM")?;
        let casm_definition = serde_json::to_vec(&casm_class)?;

        Ok(casm_definition)
    }

    fn to_cairo_lang(class: &starknet_core::types::FlattenedSierraClass) -> ContractClass {
        ContractClass {
            sierra_program: class.sierra_program.iter().map(felt_to_big_uint_as_hex).collect(),
            sierra_program_debug_info: None,
            contract_class_version: class.contract_class_version.clone(),
            entry_points_by_type: entry_points_by_type_to_contract_entry_points(&class.entry_points_by_type),
            abi: None,
        }
    }

    /// Converts a [EntryPointsByType] to a [ContractEntryPoints]
    fn entry_points_by_type_to_contract_entry_points(
        value: &starknet_core::types::EntryPointsByType,
    ) -> ContractEntryPoints {
        fn sierra_entry_point_to_contract_entry_point(
            value: starknet_core::types::SierraEntryPoint,
        ) -> ContractEntryPoint {
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
    use super::{felt_to_big_uint, Context, Felt};
    use casm_compiler_v1_1_1::casm_contract_class::CasmContractClass;
    use casm_compiler_v1_1_1::contract_class::{ContractClass, ContractEntryPoint, ContractEntryPoints};
    use casm_utils_v1_1_1::bigint::BigUintAsHex;

    pub(super) fn compile(sierra: &starknet_core::types::FlattenedSierraClass) -> anyhow::Result<Vec<u8>> {
        let sierra_class = to_cairo_lang(sierra);

        let casm_class = CasmContractClass::from_contract_class(sierra_class, true).context("Compiling to CASM")?;
        let casm_definition = serde_json::to_vec(&casm_class)?;

        Ok(casm_definition)
    }

    fn to_cairo_lang(class: &starknet_core::types::FlattenedSierraClass) -> ContractClass {
        ContractClass {
            sierra_program: class.sierra_program.iter().map(felt_to_big_uint_as_hex).collect(),
            sierra_program_debug_info: None,
            contract_class_version: class.contract_class_version.clone(),
            entry_points_by_type: entry_points_by_type_to_contract_entry_points(&class.entry_points_by_type),
            abi: None,
        }
    }

    /// Converts a [EntryPointsByType] to a [ContractEntryPoints]
    fn entry_points_by_type_to_contract_entry_points(
        value: &starknet_core::types::EntryPointsByType,
    ) -> ContractEntryPoints {
        fn sierra_entry_point_to_contract_entry_point(
            value: starknet_core::types::SierraEntryPoint,
        ) -> ContractEntryPoint {
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
    use super::{felt_to_big_uint, Context, Felt};
    use casm_classes_v2::casm_contract_class::CasmContractClass;
    use casm_classes_v2::contract_class::{ContractClass, ContractEntryPoint, ContractEntryPoints};
    use casm_utils_v2::bigint::BigUintAsHex;

    pub(super) fn compile(sierra: &starknet_core::types::FlattenedSierraClass) -> anyhow::Result<Vec<u8>> {
        let sierra_class = to_cairo_lang(sierra);

        let casm_class =
            CasmContractClass::from_contract_class(sierra_class, true, usize::MAX).context("Compiling to CASM")?;
        let casm_definition = serde_json::to_vec(&casm_class)?;

        Ok(casm_definition)
    }

    fn to_cairo_lang(class: &starknet_core::types::FlattenedSierraClass) -> ContractClass {
        ContractClass {
            sierra_program: class.sierra_program.iter().map(felt_to_big_uint_as_hex).collect(),
            //sierra_program: vec![],
            sierra_program_debug_info: None,
            contract_class_version: class.contract_class_version.clone(),
            entry_points_by_type: entry_points_by_type_to_contract_entry_points(&class.entry_points_by_type),
            abi: None,
        }
    }

    /// Converts a [EntryPointsByType] to a [ContractEntryPoints]
    fn entry_points_by_type_to_contract_entry_points(
        value: &starknet_core::types::EntryPointsByType,
    ) -> ContractEntryPoints {
        fn sierra_entry_point_to_contract_entry_point(
            value: starknet_core::types::SierraEntryPoint,
        ) -> ContractEntryPoint {
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

    use crate::to_blockifier_class;

    use super::*;
    use starknet_core::types::BlockId;
    use starknet_core::types::BlockTag;
    use starknet_providers::{Provider, SequencerGatewayProvider};
    use starknet_types_core::felt::Felt;

    #[tokio::test]
    async fn test_legacy_contract_class_blockifier() {
        let provider = SequencerGatewayProvider::starknet_alpha_mainnet();
        let class_hash = Felt::from_hex_unchecked("0x25ec026985a3bf9d0cc1fe17326b245dfdc3ff89b8fde106542a3ea56c5a918");
        let class = provider.get_class(BlockId::Tag(BlockTag::Latest), class_hash).await.unwrap();

        let starknet_core::types::ContractClass::Legacy(_) = class else { panic!("Not a Legacy contract") };

        let compiled_class = class.compile().unwrap();
        to_blockifier_class(compiled_class).unwrap();
    }

    #[tokio::test]
    async fn test_compressed_sierra_contract_class_to_json() {
        let provider = SequencerGatewayProvider::starknet_alpha_mainnet();

        let class_hash = Felt::from_hex_unchecked("0x816dd0297efc55dc1e7559020a3a825e81ef734b558f03c83325d4da7e6253");
        let class = provider.get_class(BlockId::Tag(BlockTag::Latest), class_hash).await.unwrap();

        let starknet_core::types::ContractClass::Sierra(_) = class else { panic!("Not a Sierra contract") };

        let start = std::time::Instant::now();
        let compiled_class = class.compile().unwrap();
        println!("compile time: {:?}", start.elapsed());

        let start = std::time::Instant::now();
        to_blockifier_class(compiled_class).unwrap();
        println!("to_blockifier_class time: {:?}", start.elapsed());
    }
}
