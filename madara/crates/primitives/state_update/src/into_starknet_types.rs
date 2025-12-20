use crate::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, PendingStateUpdate,
    ReplacedClassItem, StateDiff, StateUpdate, StorageEntry,
};

impl From<mp_rpc::v0_7_1::StateUpdate> for StateUpdate {
    fn from(state_update: mp_rpc::v0_7_1::StateUpdate) -> Self {
        Self {
            block_hash: state_update.block_hash,
            old_root: state_update.old_root,
            new_root: state_update.new_root,
            state_diff: state_update.state_diff.into(),
        }
    }
}

impl From<StateUpdate> for mp_rpc::v0_7_1::StateUpdate {
    fn from(state_update: StateUpdate) -> Self {
        Self {
            block_hash: state_update.block_hash,
            old_root: state_update.old_root,
            new_root: state_update.new_root,
            state_diff: state_update.state_diff.into(),
        }
    }
}

impl From<mp_rpc::v0_7_1::PendingStateUpdate> for PendingStateUpdate {
    fn from(pending_state_update: mp_rpc::v0_7_1::PendingStateUpdate) -> Self {
        Self { old_root: pending_state_update.old_root, state_diff: pending_state_update.state_diff.into() }
    }
}

impl From<PendingStateUpdate> for mp_rpc::v0_7_1::PendingStateUpdate {
    fn from(pending_state_update: PendingStateUpdate) -> Self {
        Self { old_root: pending_state_update.old_root, state_diff: pending_state_update.state_diff.into() }
    }
}

impl From<mp_rpc::v0_7_1::StateDiff> for StateDiff {
    fn from(state_diff: mp_rpc::v0_7_1::StateDiff) -> Self {
        Self {
            storage_diffs: state_diff.storage_diffs.into_iter().map(|diff| diff.into()).collect(),
            old_declared_contracts: state_diff.deprecated_declared_classes,
            declared_classes: state_diff
                .declared_classes
                .into_iter()
                .map(|declared_class| declared_class.into())
                .collect(),
            deployed_contracts: state_diff
                .deployed_contracts
                .into_iter()
                .map(|deployed_contract| deployed_contract.into())
                .collect(),
            replaced_classes: state_diff
                .replaced_classes
                .into_iter()
                .map(|replaced_class| replaced_class.into())
                .collect(),
            nonces: state_diff.nonces.into_iter().map(|nonce| nonce.into()).collect(),
            migrated_compiled_classes: vec![], // v0.7.1 doesn't have migrated classes
        }
    }
}

impl From<StateDiff> for mp_rpc::v0_7_1::StateDiff {
    fn from(state_diff: StateDiff) -> Self {
        Self {
            storage_diffs: state_diff.storage_diffs.into_iter().map(|diff| diff.into()).collect(),
            deprecated_declared_classes: state_diff.old_declared_contracts,
            declared_classes: state_diff
                .declared_classes
                .into_iter()
                .map(|declared_class| declared_class.into())
                .collect(),
            deployed_contracts: state_diff
                .deployed_contracts
                .into_iter()
                .map(|deployed_contract| deployed_contract.into())
                .collect(),
            replaced_classes: state_diff
                .replaced_classes
                .into_iter()
                .map(|replaced_class| replaced_class.into())
                .collect(),
            nonces: state_diff.nonces.into_iter().map(|nonce| nonce.into()).collect(),
        }
    }
}

impl From<mp_rpc::v0_7_1::ContractStorageDiffItem> for ContractStorageDiffItem {
    fn from(contract_storage_diff_item: mp_rpc::v0_7_1::ContractStorageDiffItem) -> Self {
        Self {
            address: contract_storage_diff_item.address,
            storage_entries: contract_storage_diff_item.storage_entries.into_iter().map(|entry| entry.into()).collect(),
        }
    }
}

impl From<ContractStorageDiffItem> for mp_rpc::v0_7_1::ContractStorageDiffItem {
    fn from(contract_storage_diff_item: ContractStorageDiffItem) -> Self {
        Self {
            address: contract_storage_diff_item.address,
            storage_entries: contract_storage_diff_item.storage_entries.into_iter().map(|entry| entry.into()).collect(),
        }
    }
}

impl From<mp_rpc::v0_7_1::KeyValuePair> for StorageEntry {
    fn from(storage_entry: mp_rpc::v0_7_1::KeyValuePair) -> Self {
        Self { key: storage_entry.key, value: storage_entry.value }
    }
}

impl From<StorageEntry> for mp_rpc::v0_7_1::KeyValuePair {
    fn from(storage_entry: StorageEntry) -> Self {
        Self { key: storage_entry.key, value: storage_entry.value }
    }
}

impl From<mp_rpc::v0_7_1::NewClasses> for DeclaredClassItem {
    fn from(declared_class_item: mp_rpc::v0_7_1::NewClasses) -> Self {
        Self {
            class_hash: declared_class_item.class_hash,
            compiled_class_hash: declared_class_item.compiled_class_hash,
        }
    }
}

impl From<DeclaredClassItem> for mp_rpc::v0_7_1::NewClasses {
    fn from(declared_class_item: DeclaredClassItem) -> Self {
        Self {
            class_hash: declared_class_item.class_hash,
            compiled_class_hash: declared_class_item.compiled_class_hash,
        }
    }
}

impl From<mp_rpc::v0_7_1::DeployedContractItem> for DeployedContractItem {
    fn from(deployed_contract_item: mp_rpc::v0_7_1::DeployedContractItem) -> Self {
        Self { address: deployed_contract_item.address, class_hash: deployed_contract_item.class_hash }
    }
}

impl From<DeployedContractItem> for mp_rpc::v0_7_1::DeployedContractItem {
    fn from(deployed_contract_item: DeployedContractItem) -> Self {
        Self { address: deployed_contract_item.address, class_hash: deployed_contract_item.class_hash }
    }
}

impl From<mp_rpc::v0_7_1::ReplacedClass> for ReplacedClassItem {
    fn from(replaced_class_item: mp_rpc::v0_7_1::ReplacedClass) -> Self {
        Self { contract_address: replaced_class_item.contract_address, class_hash: replaced_class_item.class_hash }
    }
}

impl From<ReplacedClassItem> for mp_rpc::v0_7_1::ReplacedClass {
    fn from(replaced_class_item: ReplacedClassItem) -> Self {
        Self { contract_address: replaced_class_item.contract_address, class_hash: replaced_class_item.class_hash }
    }
}

impl From<mp_rpc::v0_7_1::NonceUpdate> for NonceUpdate {
    fn from(nonce_update: mp_rpc::v0_7_1::NonceUpdate) -> Self {
        Self { contract_address: nonce_update.contract_address, nonce: nonce_update.nonce }
    }
}

impl From<NonceUpdate> for mp_rpc::v0_7_1::NonceUpdate {
    fn from(nonce_update: NonceUpdate) -> Self {
        Self { contract_address: nonce_update.contract_address, nonce: nonce_update.nonce }
    }
}

// v0.10.0 conversions
impl From<StateDiff> for mp_rpc::v0_10_0::StateDiff {
    fn from(state_diff: StateDiff) -> Self {
        Self {
            storage_diffs: state_diff
                .storage_diffs
                .into_iter()
                .map(|diff| mp_rpc::v0_10_0::ContractStorageDiffItem {
                    address: diff.address,
                    storage_entries: diff
                        .storage_entries
                        .into_iter()
                        .map(|entry| mp_rpc::v0_10_0::KeyValuePair { key: entry.key, value: entry.value })
                        .collect(),
                })
                .collect(),
            deprecated_declared_classes: state_diff.old_declared_contracts,
            declared_classes: state_diff
                .declared_classes
                .into_iter()
                .map(|item| mp_rpc::v0_10_0::NewClasses {
                    class_hash: item.class_hash,
                    compiled_class_hash: item.compiled_class_hash,
                })
                .collect(),
            deployed_contracts: state_diff
                .deployed_contracts
                .into_iter()
                .map(|item| mp_rpc::v0_10_0::DeployedContractItem {
                    address: item.address,
                    class_hash: item.class_hash,
                })
                .collect(),
            replaced_classes: state_diff
                .replaced_classes
                .into_iter()
                .map(|item| mp_rpc::v0_10_0::ReplacedClass {
                    contract_address: item.contract_address,
                    class_hash: item.class_hash,
                })
                .collect(),
            nonces: state_diff
                .nonces
                .into_iter()
                .map(|item| mp_rpc::v0_10_0::NonceUpdate { contract_address: item.contract_address, nonce: item.nonce })
                .collect(),
            migrated_compiled_classes: state_diff
                .migrated_compiled_classes
                .into_iter()
                .map(|item| mp_rpc::v0_10_0::MigratedClassItem {
                    class_hash: item.class_hash,
                    compiled_class_hash: item.compiled_class_hash,
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use mp_convert::test::assert_consistent_conversion;
    use starknet_types_core::felt::Felt;

    use crate::tests::dummy_state_diff;

    #[test]
    fn test_state_update_core_convertion() {
        let state_update = StateUpdate {
            block_hash: Felt::from_hex_unchecked("0x1234"),
            old_root: Felt::from_hex_unchecked("0x5678"),
            new_root: Felt::from_hex_unchecked("0x9abc"),
            state_diff: dummy_state_diff(),
        };

        assert_consistent_conversion::<_, mp_rpc::v0_7_1::StateUpdate>(state_update);
    }

    #[test]
    fn test_pending_state_update_core_convertion() {
        let pending_state_update =
            PendingStateUpdate { old_root: Felt::from_hex_unchecked("0x5678"), state_diff: dummy_state_diff() };

        assert_consistent_conversion::<_, mp_rpc::v0_7_1::PendingStateUpdate>(pending_state_update);
    }
}
