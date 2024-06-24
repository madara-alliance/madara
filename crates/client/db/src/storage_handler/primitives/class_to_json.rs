use std::io::{Cursor, Read};

use anyhow::Context;

pub trait ClassToJson {
    fn to_json(&self) -> anyhow::Result<Vec<u8>>;
}

impl ClassToJson for starknet_core::types::ContractClass {
    fn to_json(&self) -> anyhow::Result<Vec<u8>> {
        match self {
            starknet_core::types::ContractClass::Sierra(class) => class.to_json(),
            starknet_core::types::ContractClass::Legacy(class) => class.to_json(),
        }
    }
}

impl ClassToJson for starknet_core::types::CompressedLegacyContractClass {
    fn to_json(&self) -> anyhow::Result<Vec<u8>> {
        // decode program
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

        Ok(serialized)
    }
}

impl ClassToJson for starknet_core::types::FlattenedSierraClass {
    fn to_json(&self) -> anyhow::Result<Vec<u8>> {
        let json = serde_json::to_vec(self)?;

        Ok(json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use starknet_core::types::BlockId;
    use starknet_core::types::BlockTag;
    use starknet_core::types::Felt;
    use starknet_providers::{Provider, SequencerGatewayProvider};

    #[tokio::test]
    async fn test_compressed_legacy_contract_class_to_json() {
        let provider = SequencerGatewayProvider::starknet_alpha_mainnet();

        let class_hash = Felt::from_hex_unchecked("0x25ec026985a3bf9d0cc1fe17326b245dfdc3ff89b8fde106542a3ea56c5a918");

        let class = provider.get_class(BlockId::Tag(BlockTag::Latest), class_hash).await.unwrap();

        let json = class.to_json().unwrap();
        let definition = String::from_utf8(json).unwrap();
        blockifier::execution::contract_class::ContractClassV0::try_from_json_string(&definition).unwrap();
    }

    #[tokio::test]
    async fn test_compressed_sierra_contract_class_to_json() {
        let provider = SequencerGatewayProvider::starknet_alpha_mainnet();

        let class_hash = Felt::from_hex_unchecked("0x29927c8af6bccf3f6fda035981e765a7bdbf18a2dc0d630494f8758aa908e2b");

        let class = provider.get_class(BlockId::Tag(BlockTag::Latest), class_hash).await.unwrap();

        let _json = class.to_json().unwrap();
    }
}
