use starknet_types_core::felt::Felt;

use crate::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, PendingStateUpdate,
    ReplacedClassItem, StateDiff, StateUpdate, StorageEntry,
};

impl From<starknet_types_rpc::StateUpdate<Felt>> for StateUpdate {
    fn from(state_update: starknet_types_rpc::StateUpdate<Felt>) -> Self {
        Self {
            block_hash: state_update.block_hash,
            old_root: state_update.old_root,
            new_root: state_update.new_root,
            state_diff: state_update.state_diff.into(),
        }
    }
}

impl From<StateUpdate> for starknet_types_rpc::StateUpdate<Felt> {
    fn from(state_update: StateUpdate) -> Self {
        Self {
            block_hash: state_update.block_hash,
            old_root: state_update.old_root,
            new_root: state_update.new_root,
            state_diff: state_update.state_diff.into(),
        }
    }
}

impl From<starknet_types_rpc::PendingStateUpdate<Felt>> for PendingStateUpdate {
    fn from(pending_state_update: starknet_types_rpc::PendingStateUpdate<Felt>) -> Self {
        Self { old_root: pending_state_update.old_root, state_diff: pending_state_update.state_diff.into() }
    }
}

impl From<PendingStateUpdate> for starknet_types_rpc::PendingStateUpdate<Felt> {
    fn from(pending_state_update: PendingStateUpdate) -> Self {
        Self { old_root: pending_state_update.old_root, state_diff: pending_state_update.state_diff.into() }
    }
}

impl From<starknet_types_rpc::StateDiff<Felt>> for StateDiff {
    fn from(state_diff: starknet_types_rpc::StateDiff<Felt>) -> Self {
        Self {
            storage_diffs: state_diff.storage_diffs.into_iter().map(|diff| diff.into()).collect(),
            deprecated_declared_classes: state_diff.deprecated_declared_classes,
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

impl From<StateDiff> for starknet_types_rpc::StateDiff<Felt> {
    fn from(state_diff: StateDiff) -> Self {
        Self {
            storage_diffs: state_diff.storage_diffs.into_iter().map(|diff| diff.into()).collect(),
            deprecated_declared_classes: state_diff.deprecated_declared_classes,
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

impl From<starknet_types_rpc::ContractStorageDiffItem<Felt>> for ContractStorageDiffItem {
    fn from(contract_storage_diff_item: starknet_types_rpc::ContractStorageDiffItem<Felt>) -> Self {
        Self {
            address: contract_storage_diff_item.address,
            storage_entries: contract_storage_diff_item.storage_entries.into_iter().map(|entry| entry.into()).collect(),
        }
    }
}

impl From<ContractStorageDiffItem> for starknet_types_rpc::ContractStorageDiffItem<Felt> {
    fn from(contract_storage_diff_item: ContractStorageDiffItem) -> Self {
        Self {
            address: contract_storage_diff_item.address,
            storage_entries: contract_storage_diff_item.storage_entries.into_iter().map(|entry| entry.into()).collect(),
        }
    }
}

impl From<starknet_types_rpc::KeyValuePair<Felt>> for StorageEntry {
    fn from(storage_entry: starknet_types_rpc::KeyValuePair<Felt>) -> Self {
        Self { key: storage_entry.key, value: storage_entry.value }
    }
}

impl From<StorageEntry> for starknet_types_rpc::KeyValuePair<Felt> {
    fn from(storage_entry: StorageEntry) -> Self {
        Self { key: storage_entry.key, value: storage_entry.value }
    }
}

impl From<starknet_types_rpc::NewClasses<Felt>> for DeclaredClassItem {
    fn from(declared_class_item: starknet_types_rpc::NewClasses<Felt>) -> Self {
        Self {
            class_hash: declared_class_item.class_hash,
            compiled_class_hash: declared_class_item.compiled_class_hash,
        }
    }
}

impl From<DeclaredClassItem> for starknet_types_rpc::NewClasses<Felt> {
    fn from(declared_class_item: DeclaredClassItem) -> Self {
        Self {
            class_hash: declared_class_item.class_hash,
            compiled_class_hash: declared_class_item.compiled_class_hash,
        }
    }
}

impl From<starknet_types_rpc::DeployedContractItem<Felt>> for DeployedContractItem {
    fn from(deployed_contract_item: starknet_types_rpc::DeployedContractItem<Felt>) -> Self {
        Self { address: deployed_contract_item.address, class_hash: deployed_contract_item.class_hash }
    }
}

impl From<DeployedContractItem> for starknet_types_rpc::DeployedContractItem<Felt> {
    fn from(deployed_contract_item: DeployedContractItem) -> Self {
        Self { address: deployed_contract_item.address, class_hash: deployed_contract_item.class_hash }
    }
}

impl From<starknet_types_rpc::ReplacedClass<Felt>> for ReplacedClassItem {
    fn from(replaced_class_item: starknet_types_rpc::ReplacedClass<Felt>) -> Self {
        Self { contract_address: replaced_class_item.contract_address, class_hash: replaced_class_item.class_hash }
    }
}

impl From<ReplacedClassItem> for starknet_types_rpc::ReplacedClass<Felt> {
    fn from(replaced_class_item: ReplacedClassItem) -> Self {
        Self { contract_address: replaced_class_item.contract_address, class_hash: replaced_class_item.class_hash }
    }
}

impl From<starknet_types_rpc::NonceUpdate<Felt>> for NonceUpdate {
    fn from(nonce_update: starknet_types_rpc::NonceUpdate<Felt>) -> Self {
        Self { contract_address: nonce_update.contract_address, nonce: nonce_update.nonce }
    }
}

impl From<NonceUpdate> for starknet_types_rpc::NonceUpdate<Felt> {
    fn from(nonce_update: NonceUpdate) -> Self {
        Self { contract_address: nonce_update.contract_address, nonce: nonce_update.nonce }
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

        assert_consistent_conversion::<_, starknet_types_rpc::StateUpdate<Felt>>(state_update);
    }

    #[test]
    fn test_pending_state_update_core_convertion() {
        let pending_state_update =
            PendingStateUpdate { old_root: Felt::from_hex_unchecked("0x5678"), state_diff: dummy_state_diff() };

        assert_consistent_conversion::<_, starknet_types_rpc::PendingStateUpdate<Felt>>(pending_state_update);
    }
}
