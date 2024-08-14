mod into_starknet_core;

use starknet_types_core::{
    felt::Felt,
    hash::{Poseidon, StarkHash},
};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StateUpdate {
    pub block_hash: Felt,
    pub old_root: Felt,
    pub new_root: Felt,
    pub state_diff: StateDiff,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PendingStateUpdate {
    pub old_root: Felt,
    pub state_diff: StateDiff,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StateDiff {
    pub storage_diffs: Vec<ContractStorageDiffItem>,
    pub deprecated_declared_classes: Vec<Felt>,
    pub declared_classes: Vec<DeclaredClassItem>,
    pub deployed_contracts: Vec<DeployedContractItem>,
    pub replaced_classes: Vec<ReplacedClassItem>,
    pub nonces: Vec<NonceUpdate>,
}

impl StateDiff {
    pub fn is_empty(&self) -> bool {
        self.deployed_contracts.is_empty()
            && self.declared_classes.is_empty()
            && self.deprecated_declared_classes.is_empty()
            && self.nonces.is_empty()
            && self.replaced_classes.is_empty()
            && self.storage_diffs.is_empty()
    }

    pub fn len(&self) -> usize {
        let mut result = 0usize;
        result += self.deployed_contracts.len();
        result += self.declared_classes.len();
        result += self.deprecated_declared_classes.len();
        result += self.nonces.len();
        result += self.replaced_classes.len();

        for storage_diff in &self.storage_diffs {
            result += storage_diff.len();
        }
        result
    }

    pub fn compute_hash(&self) -> Felt {
        let updated_contracts_sorted = {
            let mut updated_contracts = self
                .deployed_contracts
                .iter()
                .map(|deployed_contract| (deployed_contract.address, deployed_contract.class_hash))
                .chain(
                    self.replaced_classes
                        .iter()
                        .map(|replaced_class| (replaced_class.contract_address, replaced_class.class_hash)),
                )
                .collect::<Vec<_>>();
            updated_contracts.sort_by_key(|(address, _)| *address);
            updated_contracts
        };

        let declared_classes_sorted = {
            let mut declared_classes = self.declared_classes.clone();
            declared_classes.sort_by_key(|declared_class| declared_class.class_hash);
            declared_classes
        };

        let deprecated_declared_classes_sorted = {
            let mut deprecated_declared_classes = self.deprecated_declared_classes.clone();
            deprecated_declared_classes.sort();
            deprecated_declared_classes
        };

        let nonces_sorted = {
            let mut nonces = self.nonces.clone();
            nonces.sort_by_key(|nonce| nonce.contract_address);
            nonces
        };

        let storage_diffs_sorted = {
            let mut storage_diffs = self.storage_diffs.clone();
            storage_diffs.sort_by_key(|storage_diff| storage_diff.address);
            storage_diffs
        };

        let updated_contracts_len_as_felt = (updated_contracts_sorted.len() as u64).into();
        let declared_classes_len_as_felt = (declared_classes_sorted.len() as u64).into();
        let deprecated_declared_classes_len_as_felt = (deprecated_declared_classes_sorted.len() as u64).into();
        let nonces_len_as_felt = (nonces_sorted.len() as u64).into();
        let storage_diffs_len_as_felt = (storage_diffs_sorted.len() as u64).into();

        let elements: Vec<Felt> = std::iter::once(Felt::from_bytes_be_slice(b"STARKNET_STATE_DIFF0"))
            .chain(std::iter::once(updated_contracts_len_as_felt))
            .chain(updated_contracts_sorted.into_iter().flat_map(|(address, class_hash)| vec![address, class_hash]))
            .chain(std::iter::once(declared_classes_len_as_felt))
            .chain(
                declared_classes_sorted
                    .into_iter()
                    .flat_map(|declared_class| vec![declared_class.class_hash, declared_class.compiled_class_hash]),
            )
            .chain(std::iter::once(deprecated_declared_classes_len_as_felt))
            .chain(deprecated_declared_classes_sorted)
            .chain(std::iter::once(Felt::ONE))
            .chain(std::iter::once(Felt::ZERO))
            .chain(std::iter::once(storage_diffs_len_as_felt))
            .chain(storage_diffs_sorted.into_iter().flat_map(|storage_diff| {
                let storage_entries_len_as_felt: Felt = (storage_diff.storage_entries.len() as u64).into();
                std::iter::once(storage_diff.address).chain(std::iter::once(storage_entries_len_as_felt)).chain(
                    storage_diff
                        .storage_entries
                        .iter()
                        .flat_map(|storage_entry| vec![storage_entry.key, storage_entry.value])
                        .collect::<Vec<_>>(),
                )
            }))
            .chain(std::iter::once(nonces_len_as_felt))
            .chain(nonces_sorted.into_iter().flat_map(|nonce| vec![nonce.contract_address, nonce.nonce]))
            .collect();

        Poseidon::hash_array(&elements)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ContractStorageDiffItem {
    pub address: Felt,
    pub storage_entries: Vec<StorageEntry>,
}

impl ContractStorageDiffItem {
    fn len(&self) -> usize {
        self.storage_entries.len()
    }
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StorageEntry {
    pub key: Felt,
    pub value: Felt,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeclaredClassItem {
    pub class_hash: Felt,
    pub compiled_class_hash: Felt,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeployedContractItem {
    pub address: Felt,
    pub class_hash: Felt,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReplacedClassItem {
    pub contract_address: Felt,
    pub class_hash: Felt,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NonceUpdate {
    pub contract_address: Felt,
    pub nonce: Felt,
}
