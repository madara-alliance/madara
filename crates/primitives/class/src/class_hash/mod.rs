mod append_to_hash_chain;
mod cairo_program;
mod starknet_keccak;

use std::io::Read;

use anyhow::{Context, Ok, Result};
use dp_hash_chain::HashChain;
use flate2::read::GzDecoder;
use starknet_core::types::{CompressedLegacyContractClass, ContractClass};
use starknet_types_core::{felt::Felt, hash::Pedersen};

use crate::class_hash::append_to_hash_chain::{AppendToHashChain, LegacyEntryPointsByTypeHashChainWrapper};

use self::{cairo_program::CairoProgram, starknet_keccak::compute_cairo_program_keccak};

pub trait ComputeClassHash {
    type Error;

    fn class_hash(&self) -> Result<Felt, Self::Error>;
}

impl ComputeClassHash for ContractClass {
    type Error = anyhow::Error;

    fn class_hash(&self) -> Result<Felt, Self::Error> {
        match self {
            ContractClass::Sierra(s) => Ok(s.class_hash()),
            ContractClass::Legacy(c) => c.class_hash(),
        }
    }
}

impl ComputeClassHash for CompressedLegacyContractClass {
    type Error = anyhow::Error;

    // Source for the implementation: pathfiner
    fn class_hash(&self) -> Result<Felt, Self::Error> {
        // Contains the de-compressed program .json file content
        let mut program_json = Vec::new();
        // A paritally deserialized program .json, that contains references to `program_json`
        // It allows for easy re-serialization into the pythonic .json format used for starknet-keccak
        let program = {
            let mut gzip_decoder = GzDecoder::new(self.program.as_slice());
            gzip_decoder
                .read_to_end(&mut program_json)
                .context("Failed to read gzip compressed self program to string")?;

            let mut program =
                serde_json::from_slice::<CairoProgram<'_>>(&program_json).context("Failed to parse program JSON")?;
            program.prepare()?;
            program
        };

        // Build hash chain
        let mut hash_chain = HashChain::<Pedersen>::default();
        hash_chain.update(&Felt::ZERO);
        // EntryPoints
        LegacyEntryPointsByTypeHashChainWrapper(&self.entry_points_by_type).append_to_hash_chain(&mut hash_chain)?;
        // Builtins
        program.builtins.append_to_hash_chain(&mut hash_chain)?;
        // Program Keccak
        hash_chain.update(&compute_cairo_program_keccak(&program, &self.abi)?);
        // Program data
        program.data.append_to_hash_chain(&mut hash_chain)?;

        Ok(hash_chain.finalize())
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
            assert_eq!(legacy.class_hash().unwrap(), class_hash);
        } else {
            panic!("Not a Lecacy contract");
        }
    }
}
