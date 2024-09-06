use crate::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, PendingStateUpdate,
    ReplacedClassItem, StateDiff, StateUpdate, StorageEntry,
};

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
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
        // sort hashmaps by key
        let mut storage_diffs = state_diff.storage_diffs.into_iter().collect::<Vec<_>>();
        storage_diffs.sort_by_key(|(address, _)| *address);

        let mut nonces = state_diff.nonces.into_iter().collect::<Vec<_>>();
        nonces.sort_by_key(|(address, _)| *address);

        Self {
            storage_diffs: storage_diffs
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
            nonces: nonces.into_iter().map(|nonce| NonceUpdate { contract_address: nonce.0, nonce: nonce.1 }).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::tests::dummy_state_diff;
    use starknet_core::types::Felt;
    use starknet_providers::sequencer::models::state_update::DeclaredContract as ProviderDeclaredContract;
    use starknet_providers::sequencer::models::state_update::DeployedContract as ProviderDeployedContract;
    use starknet_providers::sequencer::models::state_update::StateDiff as ProviderStateDiff;
    use starknet_providers::sequencer::models::state_update::StorageDiff as ProviderStorageDiff;

    #[test]
    fn test_try_from_state_update() {
        let provider_state_update = starknet_providers::sequencer::models::StateUpdate {
            block_hash: Some(Felt::from(1)),
            old_root: Felt::from(2),
            new_root: Some(Felt::from(3)),
            state_diff: ProviderStateDiff {
                storage_diffs: vec![
                    (
                        Felt::from(1),
                        vec![
                            ProviderStorageDiff { key: Felt::from(2), value: Felt::from(3) },
                            ProviderStorageDiff { key: Felt::from(4), value: Felt::from(5) },
                        ],
                    ),
                    (
                        Felt::from(6),
                        vec![
                            ProviderStorageDiff { key: Felt::from(7), value: Felt::from(8) },
                            ProviderStorageDiff { key: Felt::from(9), value: Felt::from(10) },
                        ],
                    ),
                ]
                .into_iter()
                .collect(),
                old_declared_contracts: vec![Felt::from(11), Felt::from(12)],
                declared_classes: vec![
                    ProviderDeclaredContract { class_hash: Felt::from(13), compiled_class_hash: Felt::from(14) },
                    ProviderDeclaredContract { class_hash: Felt::from(15), compiled_class_hash: Felt::from(16) },
                ],
                deployed_contracts: vec![
                    ProviderDeployedContract { address: Felt::from(17), class_hash: Felt::from(18) },
                    ProviderDeployedContract { address: Felt::from(19), class_hash: Felt::from(20) },
                ],
                replaced_classes: vec![
                    ProviderDeployedContract { address: Felt::from(21), class_hash: Felt::from(22) },
                    ProviderDeployedContract { address: Felt::from(23), class_hash: Felt::from(24) },
                ],
                nonces: vec![(Felt::from(25), Felt::from(26)), (Felt::from(27), Felt::from(28))].into_iter().collect(),
            },
        };

        let state_update = StateUpdate::try_from(provider_state_update).unwrap();
        let expected_state_update = StateUpdate {
            block_hash: Felt::from(1),
            old_root: Felt::from(2),
            new_root: Felt::from(3),
            state_diff: dummy_state_diff(),
        };
        assert_eq!(state_update, expected_state_update);
    }

    #[test]
    fn test_try_from_state_update_missing_block_hash() {
        let state_update = starknet_providers::sequencer::models::StateUpdate {
            block_hash: None,
            old_root: Felt::from(2),
            new_root: Some(Felt::from(3)),
            state_diff: empty_provider_state_diff(),
        };

        let error = StateUpdate::try_from(state_update).unwrap_err();
        assert_eq!(error, ProviderStateUpdateError::MissingBlockHash);
    }

    #[test]
    fn test_try_from_state_update_missing_new_root() {
        let state_update = starknet_providers::sequencer::models::StateUpdate {
            block_hash: Some(Felt::from(1)),
            old_root: Felt::from(2),
            new_root: None,
            state_diff: empty_provider_state_diff(),
        };

        let error = StateUpdate::try_from(state_update).unwrap_err();
        assert_eq!(error, ProviderStateUpdateError::MissingNewRoot);
    }

    fn empty_provider_state_diff() -> ProviderStateDiff {
        ProviderStateDiff {
            storage_diffs: HashMap::new(),
            old_declared_contracts: vec![],
            declared_classes: vec![],
            deployed_contracts: vec![],
            replaced_classes: vec![],
            nonces: HashMap::new(),
        }
    }
}
