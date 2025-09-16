use starknet_types_core::{
    felt::Felt,
    hash::{Poseidon, StarkHash},
};
use std::collections::HashMap;
use mp_convert::ToFelt;
mod into_starknet_types;

#[derive(Copy, Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DeclaredClassCompiledClass {
    Sierra(/* compiled_class_hash */ Felt),
    Legacy,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum ClassUpdateItem {
    DeployedContract(Felt),
    ReplacedClass(Felt),
}

impl ClassUpdateItem {
    pub fn class_hash(&self) -> &Felt {
        match self {
            ClassUpdateItem::DeployedContract(class_hash) => class_hash,
            ClassUpdateItem::ReplacedClass(class_hash) => class_hash,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct TransactionStateUpdate {
    pub nonces: HashMap<Felt, Felt>,
    pub storage_diffs: HashMap<(Felt, Felt), Felt>,
    pub declared_classes: HashMap<Felt, DeclaredClassCompiledClass>,
    pub contract_class_hashes: HashMap<Felt, ClassUpdateItem>,
}

impl TransactionStateUpdate {
    // Note: you need to re-normalize after..
    #[allow(unused)]
    fn append(&mut self, state_diff: &TransactionStateUpdate) {
        self.nonces.extend(state_diff.nonces.iter());
        self.storage_diffs.extend(state_diff.storage_diffs.iter());
        self.declared_classes.extend(state_diff.declared_classes.iter());
        self.contract_class_hashes.extend(state_diff.contract_class_hashes.iter());
    }

    // Note: you need to re-normalize after..
    fn append_state_diff(&mut self, state_diff: &StateDiff) {
        self.nonces.extend(state_diff.nonces.iter().map(|entry| (entry.contract_address, entry.nonce)));
        self.storage_diffs.extend(
            state_diff
                .storage_diffs
                .iter()
                .flat_map(|entry| entry.storage_entries.iter().map(|e| ((entry.address, e.key), e.value))),
        );
        self.declared_classes.extend(
            state_diff
                .declared_classes
                .iter()
                .map(|entry| (entry.class_hash, DeclaredClassCompiledClass::Sierra(entry.compiled_class_hash)))
                .chain(
                    state_diff
                        .old_declared_contracts
                        .iter()
                        .map(|class_hash| (*class_hash, DeclaredClassCompiledClass::Legacy)),
                ),
        );
        self.contract_class_hashes.extend(
            state_diff
                .replaced_classes
                .iter()
                .map(|entry| (entry.contract_address, ClassUpdateItem::ReplacedClass(entry.class_hash)))
                .chain(
                    state_diff
                        .deployed_contracts
                        .iter()
                        .map(|entry| (entry.address, ClassUpdateItem::DeployedContract(entry.class_hash))),
                ),
        );
    }

    pub fn from_state_diff(state_diff: &StateDiff) -> Self {
        let mut this = Self::default();
        this.append_state_diff(state_diff);
        this
    }

    pub fn to_state_diff(&self) -> StateDiff {
        fn sorted_by_key<T, K: Ord, F: FnMut(&T) -> K>(mut vec: Vec<T>, f: F) -> Vec<T> {
            vec.sort_by_key(f);
            vec
        }

        let mut storage_diffs: HashMap<Felt, HashMap<Felt, Felt>> = Default::default();
        let mut declared_classes: Vec<DeclaredClassItem> = Default::default();
        let mut old_declared_contracts: Vec<Felt> = Default::default();
        let mut deployed_contracts: Vec<DeployedContractItem> = Default::default();
        let mut replaced_classes: Vec<ReplacedClassItem> = Default::default();

        for (&(contract, key), &value) in &self.storage_diffs {
            storage_diffs.entry(contract).or_default().insert(key, value);
        }

        for (&class_hash, &compiled_class_hash) in &self.declared_classes {
            match compiled_class_hash {
                DeclaredClassCompiledClass::Legacy => old_declared_contracts.push(class_hash),
                DeclaredClassCompiledClass::Sierra(compiled_class_hash) => {
                    declared_classes.push(DeclaredClassItem { class_hash, compiled_class_hash })
                }
            }
        }

        for (&contract_address, &entry) in &self.contract_class_hashes {
            match entry {
                ClassUpdateItem::DeployedContract(class_hash) => {
                    deployed_contracts.push(DeployedContractItem { address: contract_address, class_hash })
                }
                ClassUpdateItem::ReplacedClass(class_hash) => {
                    replaced_classes.push(ReplacedClassItem { contract_address, class_hash })
                }
            }
        }

        StateDiff {
            storage_diffs: sorted_by_key(
                storage_diffs
                    .into_iter()
                    .map(|(address, storage_entries)| ContractStorageDiffItem {
                        address,
                        storage_entries: sorted_by_key(
                            storage_entries.into_iter().map(|(key, value)| StorageEntry { key, value }).collect(),
                            |entry| entry.key,
                        ),
                    })
                    .collect(),
                |entry| entry.address,
            ),
            old_declared_contracts: sorted_by_key(old_declared_contracts, |&class_hash| class_hash),
            declared_classes: sorted_by_key(declared_classes, |entry| entry.class_hash),
            deployed_contracts: sorted_by_key(deployed_contracts, |entry| entry.address),
            replaced_classes: sorted_by_key(replaced_classes, |entry| entry.contract_address),
            nonces: sorted_by_key(
                self.nonces.iter().map(|(&contract_address, &nonce)| NonceUpdate { contract_address, nonce }).collect(),
                |entry| entry.contract_address,
            ),
        }
    }
}

impl From<StateDiff> for TransactionStateUpdate {
    fn from(value: StateDiff) -> Self {
        Self::from_state_diff(&value)
    }
}

impl From<TransactionStateUpdate> for StateDiff {
    fn from(value: TransactionStateUpdate) -> Self {
        value.to_state_diff()
    }
}

// Add conversion from blockifier::state::cached_state::CommitmentStateDiff
impl From<blockifier::state::cached_state::CommitmentStateDiff> for StateDiff {
    fn from(commitment_state_diff: blockifier::state::cached_state::CommitmentStateDiff) -> Self {
        let mut storage_diffs = Vec::new();
        let mut deployed_contracts = Vec::new();
        let replaced_classes = Vec::new();
        let mut declared_classes = Vec::new();
        let mut nonces = Vec::new();

        // Convert storage updates
        for (address, updates) in commitment_state_diff.storage_updates {
            let storage_entries: Vec<StorageEntry> = updates
                .into_iter()
                .map(|(key, value)| StorageEntry {
                    key: key.to_felt(),
                    value,
                })
                .collect();
            
            if !storage_entries.is_empty() {
                storage_diffs.push(ContractStorageDiffItem {
                    address: address.to_felt(),
                    storage_entries,
                });
            }
        }

        // Convert deployed contracts and replaced classes
        for (address, class_hash) in commitment_state_diff.address_to_class_hash {
            // Check if this is a new deployment or class replacement
            // For simplicity, we'll treat all as deployed contracts
            deployed_contracts.push(DeployedContractItem {
                address: address.to_felt(),
                class_hash: class_hash.to_felt(),
            });
        }

        // Convert declared classes
        for (class_hash, compiled_class_hash) in commitment_state_diff.class_hash_to_compiled_class_hash {
            declared_classes.push(DeclaredClassItem {
                class_hash: class_hash.to_felt(),
                compiled_class_hash: compiled_class_hash.to_felt(),
            });
        }

        // Convert nonces
        for (address, nonce) in commitment_state_diff.address_to_nonce {
            nonces.push(NonceUpdate {
                contract_address: address.to_felt(),
                nonce: nonce.to_felt(),
            });
        }

        StateDiff {
            storage_diffs,
            old_declared_contracts: Vec::new(), // This would need to be determined from context
            declared_classes,
            deployed_contracts,
            replaced_classes,
            nonces,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    /// Changed storage values. Mapping (contract_address, storage_key) => value.
    pub storage_diffs: Vec<ContractStorageDiffItem>,
    /// New declared classes. List of class hashes.
    pub old_declared_contracts: Vec<Felt>,
    /// New declared classes. Mapping class_hash => compiled_class_hash.
    pub declared_classes: Vec<DeclaredClassItem>,
    /// New contract. Mapping contract_address => class_hash.
    pub deployed_contracts: Vec<DeployedContractItem>,
    /// Contract has changed class. Mapping contract_address => class_hash.
    pub replaced_classes: Vec<ReplacedClassItem>,
    /// New contract nonce. Mapping contract_address => nonce.
    pub nonces: Vec<NonceUpdate>,
}

impl StateDiff {
    pub fn is_empty(&self) -> bool {
        self.deployed_contracts.is_empty()
            && self.declared_classes.is_empty()
            && self.old_declared_contracts.is_empty()
            && self.nonces.is_empty()
            && self.replaced_classes.is_empty()
            && self.storage_diffs.is_empty()
    }

    pub fn len(&self) -> usize {
        let mut result = 0usize;
        result += self.deployed_contracts.len();
        result += self.declared_classes.len();
        result += self.old_declared_contracts.len();
        result += self.nonces.len();
        result += self.replaced_classes.len();

        for storage_diff in &self.storage_diffs {
            result += storage_diff.len();
        }
        result
    }

    pub fn sort(&mut self) {
        self.storage_diffs.iter_mut().for_each(|storage_diff| storage_diff.sort_storage_entries());
        self.storage_diffs.sort_by_key(|storage_diff| storage_diff.address);
        self.old_declared_contracts.sort();
        self.declared_classes.sort_by_key(|declared_class| declared_class.class_hash);
        self.deployed_contracts.sort_by_key(|deployed_contract| deployed_contract.address);
        self.replaced_classes.sort_by_key(|replaced_class| replaced_class.contract_address);
        self.nonces.sort_by_key(|nonce| nonce.contract_address);
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
            let mut deprecated_declared_classes = self.old_declared_contracts.clone();
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
            storage_diffs.iter_mut().for_each(|storage_diff| storage_diff.sort_storage_entries());
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

    pub fn all_declared_classes(&self) -> HashMap<Felt, DeclaredClassCompiledClass> {
        self.declared_classes
            .iter()
            .map(|class| (class.class_hash, DeclaredClassCompiledClass::Sierra(class.compiled_class_hash)))
            .chain(
                self.old_declared_contracts.iter().map(|class_hash| (*class_hash, DeclaredClassCompiledClass::Legacy)),
            )
            .collect()
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

    pub fn sort_storage_entries(&mut self) {
        self.storage_entries.sort_by_key(|storage_entry| storage_entry.key);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageEntry {
    pub key: Felt,
    pub value: Felt,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclaredClassItem {
    pub class_hash: Felt,
    pub compiled_class_hash: Felt,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
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

#[cfg(test)]
mod tests {
    use starknet_types_core::felt::Felt;

    use super::*;

    #[test]
    fn test_is_empty() {
        let state_diff = StateDiff::default();
        assert!(state_diff.is_empty());

        let state_diff = StateDiff { old_declared_contracts: vec![Felt::ONE], ..Default::default() };
        assert!(!state_diff.is_empty());
    }

    #[test]
    fn test_len() {
        let state_diff = StateDiff::default();
        assert_eq!(state_diff.len(), 0);

        let state_diff = dummy_state_diff();
        assert_eq!(state_diff.len(), 14);
    }

    #[test]
    fn test_compute_hash() {
        let state_diff = dummy_state_diff();
        let hash = state_diff.compute_hash();
        assert_eq!(hash, Felt::from_hex_unchecked("0x3bda8176c564f07b91627f95e1c6249c0d19ba00e47edfc17ae52ccf946ea20"));
    }

    #[test]
    fn test_compute_hash_sorted() {
        let state_diff_one = dummy_state_diff();
        let mut state_diff_two = state_diff_one.clone();

        // reversting all vectors inside state_diff_two
        // to check if hash still matches
        state_diff_two.storage_diffs.reverse();
        for diff in state_diff_two.storage_diffs.iter_mut() {
            diff.storage_entries.reverse();
        }
        state_diff_two.old_declared_contracts.reverse();
        state_diff_two.declared_classes.reverse();
        state_diff_two.deployed_contracts.reverse();
        state_diff_two.replaced_classes.reverse();
        state_diff_two.nonces.reverse();

        assert_eq!(state_diff_one.compute_hash(), state_diff_two.compute_hash());
    }

    pub(crate) fn dummy_state_diff() -> StateDiff {
        StateDiff {
            storage_diffs: vec![
                ContractStorageDiffItem {
                    address: Felt::from(1),
                    storage_entries: vec![
                        StorageEntry { key: Felt::from(2), value: Felt::from(3) },
                        StorageEntry { key: Felt::from(4), value: Felt::from(5) },
                    ],
                },
                ContractStorageDiffItem {
                    address: Felt::from(6),
                    storage_entries: vec![
                        StorageEntry { key: Felt::from(7), value: Felt::from(8) },
                        StorageEntry { key: Felt::from(9), value: Felt::from(10) },
                    ],
                },
            ],
            old_declared_contracts: vec![Felt::from(11), Felt::from(12)],
            declared_classes: vec![
                DeclaredClassItem { class_hash: Felt::from(13), compiled_class_hash: Felt::from(14) },
                DeclaredClassItem { class_hash: Felt::from(15), compiled_class_hash: Felt::from(16) },
            ],
            deployed_contracts: vec![
                DeployedContractItem { address: Felt::from(17), class_hash: Felt::from(18) },
                DeployedContractItem { address: Felt::from(19), class_hash: Felt::from(20) },
            ],
            replaced_classes: vec![
                ReplacedClassItem { contract_address: Felt::from(21), class_hash: Felt::from(22) },
                ReplacedClassItem { contract_address: Felt::from(23), class_hash: Felt::from(24) },
            ],
            nonces: vec![
                NonceUpdate { contract_address: Felt::from(25), nonce: Felt::from(26) },
                NonceUpdate { contract_address: Felt::from(27), nonce: Felt::from(28) },
            ],
        }
    }
}
