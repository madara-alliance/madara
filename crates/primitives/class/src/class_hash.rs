use anyhow::{Context, Error, Ok, Result};
use flate2::read::GzDecoder;
use serde::Serialize;
use serde_json::{Map, Value};
use sha3::{Digest, Keccak256};
use starknet_core::types::{
    contract::legacy::{
        LegacyContractClass, LegacyEntrypointOffset, LegacyProgram, RawLegacyEntryPoint, RawLegacyEntryPoints,
    },
    CompressedLegacyContractClass, ContractClass, LegacyContractEntryPoint,
};
use starknet_types_core::{
    felt::Felt,
    hash::{Pedersen, StarkHash},
};
use std::io::Read;

pub trait ClassHash {
    fn class_hash(&self) -> anyhow::Result<Felt>;
}

impl ClassHash for ContractClass {
    fn class_hash(&self) -> anyhow::Result<Felt> {
        match self {
            ContractClass::Sierra(sierra) => Ok(sierra.class_hash()),
            ContractClass::Legacy(legacy) => legacy.class_hash(),
        }
    }
}

// Define the HashChain struct
#[derive(Default)]
pub struct HashChain {
    hash: Felt,
    count: usize,
}

impl HashChain {
    pub fn update(&mut self, value: &Felt) {
        // Replace this with the actual Pedersen hash function implementation
        self.hash = Pedersen::hash(&self.hash, value);
        self.count += 1;
    }

    pub fn finalize(self) -> Felt {
        // Replace this with the actual Pedersen hash function implementation
        Pedersen::hash(&self.hash, &Felt::from(self.count))
    }
}

impl ClassHash for CompressedLegacyContractClass {
    fn class_hash(&self) -> anyhow::Result<Felt> {
        let mut contract_definition = parse_compressed_legacy_class(self.clone())?;
        contract_definition.program.debug_info = None;

        if let Some(attributes) = &mut contract_definition.program.attributes {
            attributes.iter_mut().try_for_each(|attr| -> anyhow::Result<()> {
                if attr.accessible_scopes.is_empty() {
                    attr.accessible_scopes.clear();
                }
                if attr.flow_tracking_data.is_none() {
                    attr.flow_tracking_data = None;
                }
                Ok(())
            })?;
        }

        fn add_extra_space_to_legacy_named_tuples(value: &mut Value) {
            match value {
                Value::Array(v) => walk_array(v),
                Value::Object(m) => walk_map(m),
                _ => {}
            }
        }

        fn walk_array(array: &mut [Value]) {
            for v in array.iter_mut() {
                add_extra_space_to_legacy_named_tuples(v);
            }
        }

        fn walk_map(object: &mut Map<String, Value>) {
            for (k, v) in object.iter_mut() {
                match v {
                    Value::String(s) => {
                        let new_value = add_extra_space_to_named_tuple_type_definition(k, s);
                        if new_value.as_ref() != s {
                            *v = Value::String(new_value.into());
                        }
                    }
                    _ => add_extra_space_to_legacy_named_tuples(v),
                }
            }
        }

        fn add_extra_space_to_named_tuple_type_definition<'a>(key: &str, value: &'a str) -> std::borrow::Cow<'a, str> {
            use std::borrow::Cow::*;
            match key {
                "cairo_type" | "value" => Owned(add_extra_space_before_colon(value)),
                _ => Borrowed(value),
            }
        }

        fn add_extra_space_before_colon(v: &str) -> String {
            v.replace(": ", " : ").replace("  :", " :")
        }

        if contract_definition.program.compiler_version.is_none() {
            let mut identifiers_value = serde_json::to_value(&mut contract_definition.program.identifiers)?;
            add_extra_space_to_legacy_named_tuples(&mut identifiers_value);
            contract_definition.program.identifiers = serde_json::from_value(identifiers_value)?;

            let mut reference_manager_value = serde_json::to_value(&mut contract_definition.program.reference_manager)?;
            add_extra_space_to_legacy_named_tuples(&mut reference_manager_value);
            contract_definition.program.reference_manager = serde_json::from_value(reference_manager_value)?;
        }

        let truncated_keccak = {
            use std::io::Write;

            let mut string_buffer = vec![];
            let mut ser = serde_json::Serializer::new(&mut string_buffer);
            contract_definition.serialize(&mut ser).context("Serializing contract_definition for Keccak256")?;

            let raw_json_output = String::from_utf8(string_buffer)?;

            let mut keccak = Keccak256::new();
            keccak.write_all(raw_json_output.as_bytes()).expect("writing to Keccak256 never fails");

            Felt::from_bytes_be_slice(&keccak.finalize())
        };

        const API_VERSION: Felt = Felt::ZERO;

        let mut outer = HashChain::default();
        outer.update(&API_VERSION);

        ["constructor", "external", "l1_handler"]
            .iter()
            .map(|key| {
                let empty_vec = Vec::new();
                let entry_points = match *key {
                    "constructor" => &contract_definition.entry_points_by_type.constructor,
                    "external" => &contract_definition.entry_points_by_type.external,
                    "l1_handler" => &contract_definition.entry_points_by_type.l1_handler,
                    _ => &empty_vec,
                };

                entry_points
                    .iter()
                    .flat_map(|x| {
                        [
                            x.selector,
                            match x.offset {
                                LegacyEntrypointOffset::U64AsHex(v) => Felt::from(v),
                                LegacyEntrypointOffset::U64AsInt(v) => Felt::from(v),
                            },
                        ]
                        .into_iter()
                    })
                    .fold(HashChain::default(), |mut hc, next| {
                        hc.update(&next);
                        hc
                    })
            })
            .for_each(|x| outer.update(&x.finalize()));

        fn update_hash_chain(mut hc: HashChain, next: &Felt) -> Result<HashChain, Error> {
            hc.update(next);
            Ok(hc)
        }

        let builtins = contract_definition
            .program
            .builtins
            .iter()
            .map(|s| Felt::from_bytes_be_slice(s.as_bytes()))
            .try_fold(HashChain::default(), |acc, item| update_hash_chain(acc, &item))
            .context("Failed to process contract_definition.program.builtins")?;

        outer.update(&builtins.finalize());
        outer.update(&truncated_keccak);

        let bytecodes = contract_definition
            .program
            .data
            .iter()
            .try_fold(HashChain::default(), update_hash_chain)
            .context("Failed to process contract_definition.program.data")?;

        outer.update(&bytecodes.finalize());

        Ok(outer.finalize())
    }
}

pub fn parse_compressed_legacy_class(class: CompressedLegacyContractClass) -> Result<LegacyContractClass> {
    let mut gzip_decoder = GzDecoder::new(class.program.as_slice());
    let mut program_json = String::new();
    gzip_decoder.read_to_string(&mut program_json).context("Failed to read gzip compressed class program to string")?;

    let program = serde_json::from_str::<LegacyProgram>(&program_json).context("Failed to parse program JSON")?;

    let is_pre_0_11_0 = match &program.compiler_version {
        Some(compiler_version) => {
            let minor_version = compiler_version
                .split('.')
                .nth(1)
                .ok_or_else(|| anyhow::anyhow!("Unexpected legacy compiler version string"))?;

            let minor_version: u8 = minor_version.parse().context("Failed to parse minor version")?;
            minor_version < 11
        }
        None => true,
    };

    let abi = match class.abi {
        Some(abi) => abi.into_iter().map(|item| item.into()).collect(),
        None => vec![],
    };

    Ok(LegacyContractClass {
        abi: Some(abi),
        entry_points_by_type: RawLegacyEntryPoints {
            constructor: class
                .entry_points_by_type
                .constructor
                .into_iter()
                .map(|item| parse_legacy_entrypoint(&item, is_pre_0_11_0))
                .collect(),
            external: class
                .entry_points_by_type
                .external
                .into_iter()
                .map(|item| parse_legacy_entrypoint(&item, is_pre_0_11_0))
                .collect(),
            l1_handler: class
                .entry_points_by_type
                .l1_handler
                .into_iter()
                .map(|item| parse_legacy_entrypoint(&item, is_pre_0_11_0))
                .collect(),
        },
        program,
    })
}

fn parse_legacy_entrypoint(entrypoint: &LegacyContractEntryPoint, pre_0_11_0: bool) -> RawLegacyEntryPoint {
    RawLegacyEntryPoint {
        // This doesn't really matter as it doesn't affect class hashes. We simply try to guess as
        // close as possible.
        offset: if pre_0_11_0 {
            LegacyEntrypointOffset::U64AsHex(entrypoint.offset)
        } else {
            LegacyEntrypointOffset::U64AsInt(entrypoint.offset)
        },
        selector: entrypoint.selector,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use starknet_core::types::BlockId;
    use starknet_core::types::BlockTag;
    use starknet_core::types::ContractClass;
    use starknet_providers::{Provider, SequencerGatewayProvider};
    use starknet_types_core::felt::Felt;

    #[tokio::test]
    #[ignore]
    async fn test_sierra_compute_class_hash() {
        let provider = SequencerGatewayProvider::starknet_alpha_mainnet();

        let class_hash = Felt::from_hex_unchecked("0x816dd0297efc55dc1e7559020a3a825e81ef734b558f03c83325d4da7e6253");

        let class = provider.get_class(BlockId::Tag(BlockTag::Latest), class_hash).await.unwrap();

        if let ContractClass::Sierra(sierra) = class {
            assert_eq!(sierra.class_hash(), class_hash);
        } else {
            panic!("Not a Sierra contract");
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_legacy_compute_class_hash() {
        let provider = SequencerGatewayProvider::starknet_alpha_mainnet();

        let class_hash = Felt::from_hex_unchecked("0x25ec026985a3bf9d0cc1fe17326b245dfdc3ff89b8fde106542a3ea56c5a918");

        let class = provider.get_class(BlockId::Tag(BlockTag::Latest), class_hash).await.unwrap();

        if let ContractClass::Legacy(legacy) = class {
            assert_eq!(legacy.class_hash().unwrap(), class_hash);
        } else {
            panic!("Not a Lecacy contract");
        }
    }
}
