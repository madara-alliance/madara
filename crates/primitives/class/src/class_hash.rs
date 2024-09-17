use starknet_types_core::{
    felt::Felt,
    hash::{Poseidon, StarkHash},
};

use crate::{
    convert::{parse_compressed_legacy_class, ParseCompressedLegacyClassError},
    CompressedLegacyContractClass, ContractClass, FlattenedSierraClass, SierraEntryPoint,
};
use starknet_core::types::contract::ComputeClassHashError as StarknetComputeClassHashError;

#[derive(Debug, thiserror::Error)]
pub enum ComputeClassHashError {
    #[error("Unsupported Sierra version: {0}")]
    UnsupportedSierraVersion(String),
    #[error(transparent)]
    StarknetError(#[from] StarknetComputeClassHashError),
    #[error(transparent)]
    ParseError(#[from] ParseCompressedLegacyClassError),
}

impl ContractClass {
    pub fn compute_class_hash(&self) -> Result<Felt, ComputeClassHashError> {
        match self {
            ContractClass::Sierra(sierra) => sierra.compute_class_hash(),
            ContractClass::Legacy(legacy) => legacy.compute_class_hash(),
        }
    }
}

const SIERRA_VERSION: Felt = Felt::from_hex_unchecked("0x434f4e54524143545f434c4153535f56302e312e30"); //b"CONTRACT_CLASS_V0.1.0"

impl FlattenedSierraClass {
    pub fn compute_class_hash(&self) -> Result<Felt, ComputeClassHashError> {
        if self.contract_class_version != "0.1.0" {
            return Err(ComputeClassHashError::UnsupportedSierraVersion(self.contract_class_version.clone()));
        }

        let external_hash = compute_hash_entries_point(&self.entry_points_by_type.external);
        let l1_handler_hash = compute_hash_entries_point(&self.entry_points_by_type.l1_handler);
        let constructor_hash = compute_hash_entries_point(&self.entry_points_by_type.constructor);
        let abi_hash = starknet_core::utils::starknet_keccak(self.abi.as_bytes());
        let program_hash = Poseidon::hash_array(&self.sierra_program);

        Ok(Poseidon::hash_array(&[
            SIERRA_VERSION,
            external_hash,
            l1_handler_hash,
            constructor_hash,
            abi_hash,
            program_hash,
        ]))
    }
}

fn compute_hash_entries_point(entry_points: &[SierraEntryPoint]) -> Felt {
    let entry_pointfalten: Vec<_> = entry_points
        .iter()
        .flat_map(|SierraEntryPoint { selector, function_idx }| [*selector, Felt::from(*function_idx)].into_iter())
        .collect();
    Poseidon::hash_array(&entry_pointfalten)
}

impl CompressedLegacyContractClass {
    pub fn compute_class_hash(&self) -> Result<Felt, ComputeClassHashError> {
        let legacy_contract_class = parse_compressed_legacy_class(self.clone().into())?;
        legacy_contract_class.class_hash().map_err(ComputeClassHashError::from)
    }
}

#[cfg(test)]
mod tests {
    use starknet_core::types::BlockId;
    use starknet_core::types::BlockTag;
    use starknet_core::types::Felt;
    use starknet_providers::{Provider, SequencerGatewayProvider};

    use crate::ContractClass;

    #[tokio::test]
    async fn test_compute_sierra_class_hash() {
        let provider = SequencerGatewayProvider::starknet_alpha_mainnet();

        let class_hash = Felt::from_hex_unchecked("0x816dd0297efc55dc1e7559020a3a825e81ef734b558f03c83325d4da7e6253");

        let class = provider.get_class(BlockId::Tag(BlockTag::Latest), class_hash).await.unwrap();

        let starknet_core::types::ContractClass::Sierra(_) = class else { panic!("Not a Sierra contract") };

        let class: ContractClass = class.into();

        let start = std::time::Instant::now();
        let computed_class_hash = class.compute_class_hash().unwrap();

        println!("computed_class_hash in {:?}", start.elapsed());
        assert_eq!(computed_class_hash, class_hash);
    }
}
