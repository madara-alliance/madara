use starknet_core::types::{
    contract::legacy::LegacyContractClass, CompressedLegacyContractClass, ContractClass, FlattenedSierraClass,
};
use starknet_types_core::felt::Felt;
use std::ops::Deref;

use crate::ToCompiledClass;

pub trait ClassHash {
    fn class_hash(&self) -> Felt;
}

impl ClassHash for ContractClass {
    fn class_hash(&self) -> Felt {
        match self {
            ContractClass::Sierra(sierra) => sierra.class_hash(),
            ContractClass::Legacy(legacy) => legacy.class_hash(),
        }
    }
}

impl ClassHash for CompressedLegacyContractClass {
    fn class_hash(&self) -> Felt {
        let compiled = self.compile().unwrap();
        let legacy_contract: LegacyContractClass = serde_json::from_slice(compiled.deref()).unwrap();
        legacy_contract.class_hash().unwrap()
    }
}

impl ClassHash for FlattenedSierraClass {
    fn class_hash(&self) -> Felt {
        self.class_hash()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use starknet_core::types::BlockId;
    use starknet_core::types::BlockTag;
    use starknet_core::types::ContractClass;
    use starknet_core::types::Felt;
    use starknet_providers::{Provider, SequencerGatewayProvider};

    #[tokio::test]
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
    async fn test_legacy_compute_class_hash() {
        let provider = SequencerGatewayProvider::starknet_alpha_mainnet();

        let class_hash = Felt::from_hex_unchecked("0x25ec026985a3bf9d0cc1fe17326b245dfdc3ff89b8fde106542a3ea56c5a918");

        let class = provider.get_class(BlockId::Tag(BlockTag::Latest), class_hash).await.unwrap();

        if let ContractClass::Legacy(legacy) = class {
            assert_eq!(legacy.class_hash(), class_hash);
        } else {
            panic!("Not a Lecacy contract");
        }
    }
}
