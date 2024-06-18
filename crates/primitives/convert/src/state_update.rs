use starknet_core::types::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, Felt, NonceUpdate, ReplacedClassItem,
    StateDiff as StateDiffCore, StateUpdate as StateUpdateCore, StorageEntry,
};
use starknet_providers::sequencer::models::StateUpdate as StateUpdateProvider;

pub trait ToStateUpdateCore {
    fn to_state_update_core(self) -> StateUpdateCore;
}

impl ToStateUpdateCore for StateUpdateProvider {
    fn to_state_update_core(self) -> StateUpdateCore {
        let block_hash = self.block_hash.unwrap_or(Felt::ZERO);
        let old_root = self.old_root;
        let new_root = self.new_root.unwrap_or(Felt::ZERO);

        let storage_diffs = self
            .state_diff
            .storage_diffs
            .into_iter()
            .map(|(address, entries)| ContractStorageDiffItem {
                address,
                storage_entries: entries
                    .into_iter()
                    .map(|entry| StorageEntry { key: entry.key, value: entry.value })
                    .collect(),
            })
            .collect();

        let deprecated_declared_classes = self.state_diff.old_declared_contracts;

        let declared_classes = self
            .state_diff
            .declared_classes
            .into_iter()
            .map(|declared_class| DeclaredClassItem {
                class_hash: declared_class.class_hash,
                compiled_class_hash: declared_class.compiled_class_hash,
            })
            .collect();

        let deployed_contracts = self
            .state_diff
            .deployed_contracts
            .into_iter()
            .map(|deployed_contract| DeployedContractItem {
                address: deployed_contract.address,
                class_hash: deployed_contract.class_hash,
            })
            .collect();

        let replaced_classes = self
            .state_diff
            .replaced_classes
            .into_iter()
            .map(|replaced_class| ReplacedClassItem {
                contract_address: replaced_class.address,
                class_hash: replaced_class.class_hash,
            })
            .collect();

        let nonces = self
            .state_diff
            .nonces
            .into_iter()
            .map(|(contract_address, nonce)| NonceUpdate { contract_address, nonce })
            .collect();

        StateUpdateCore {
            block_hash,
            old_root,
            new_root,
            state_diff: StateDiffCore {
                storage_diffs,
                deprecated_declared_classes,
                declared_classes,
                deployed_contracts,
                replaced_classes,
                nonces,
            },
        }
    }
}
