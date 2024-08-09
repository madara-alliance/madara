use crate::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, PendingStateUpdate,
    ReplacedClassItem, StateDiff, StateUpdate, StorageEntry,
};

#[derive(Debug, thiserror::Error)]
pub enum ProviderStateUpdateError {
    #[error("Missing block hash")]
    MissingBlockHash,
    #[error("Missing new root")]
    MissingNewRoot,
}

impl TryFrom<starknet_providers::sequencer::models::StateUpdate> for StateUpdate {
    type Error = ProviderStateUpdateError;

    fn try_from(state_update: starknet_providers::sequencer::models::StateUpdate) -> Result<Self, Self::Error> {
        Ok(Self {
            block_hash: state_update.block_hash.ok_or(ProviderStateUpdateError::MissingBlockHash)?,
            old_root: state_update.old_root,
            new_root: state_update.new_root.ok_or(ProviderStateUpdateError::MissingNewRoot)?,
            state_diff: state_update.state_diff.into(),
        })
    }
}

impl From<starknet_providers::sequencer::models::StateUpdate> for PendingStateUpdate {
    fn from(state_update: starknet_providers::sequencer::models::StateUpdate) -> Self {
        Self { old_root: state_update.old_root, state_diff: state_update.state_diff.into() }
    }
}

impl From<starknet_providers::sequencer::models::state_update::StateDiff> for StateDiff {
    fn from(state_diff: starknet_providers::sequencer::models::state_update::StateDiff) -> Self {
        Self {
            storage_diffs: state_diff
                .storage_diffs
                .into_iter()
                .map(|(address, entries)| ContractStorageDiffItem {
                    address,
                    storage_entries: entries
                        .into_iter()
                        .map(|entry| StorageEntry { key: entry.key, value: entry.value })
                        .collect(),
                })
                .collect(),
            deprecated_declared_classes: state_diff.old_declared_contracts,
            declared_classes: state_diff
                .declared_classes
                .into_iter()
                .map(|declared_class| DeclaredClassItem {
                    class_hash: declared_class.class_hash,
                    compiled_class_hash: declared_class.compiled_class_hash,
                })
                .collect(),
            deployed_contracts: state_diff
                .deployed_contracts
                .into_iter()
                .map(|deployed_contract| DeployedContractItem {
                    address: deployed_contract.address,
                    class_hash: deployed_contract.class_hash,
                })
                .collect(),
            replaced_classes: state_diff
                .replaced_classes
                .into_iter()
                .map(|replaced_class| ReplacedClassItem {
                    contract_address: replaced_class.address,
                    class_hash: replaced_class.class_hash,
                })
                .collect(),
            nonces: state_diff
                .nonces
                .into_iter()
                .map(|nonce| NonceUpdate { contract_address: nonce.0, nonce: nonce.1 })
                .collect(),
        }
    }
}
