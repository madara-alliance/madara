use std::collections::HashMap;

use mp_state_update::{DeclaredClassItem, DeployedContractItem, StorageEntry};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use starknet_types_core::felt::Felt;

#[derive(Debug, Clone, PartialEq, Serialize)] // no Deserialize because it's untagged
#[serde(untagged)]
pub enum ProviderMaybePendingStateUpdate {
    Update(StateUpdateProvider),
    Pending(PendingStateUpdateProvider),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StateUpdateProvider {
    pub block_hash: Felt,
    pub new_root: Felt,
    pub old_root: Felt,
    pub state_diff: StateDiff,
}

impl From<mp_state_update::StateUpdate> for StateUpdateProvider {
    fn from(state_update: mp_state_update::StateUpdate) -> Self {
        Self {
            block_hash: state_update.block_hash,
            new_root: state_update.new_root,
            old_root: state_update.old_root,
            state_diff: state_update.state_diff.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PendingStateUpdateProvider {
    pub old_root: Felt,
    pub state_diff: StateDiff,
}

impl From<mp_state_update::PendingStateUpdate> for PendingStateUpdateProvider {
    fn from(pending_state_update: mp_state_update::PendingStateUpdate) -> Self {
        Self { old_root: pending_state_update.old_root, state_diff: pending_state_update.state_diff.into() }
    }
}

#[serde_as]
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct StateDiff {
    pub storage_diffs: HashMap<Felt, Vec<StorageEntry>>,
    pub deployed_contracts: Vec<DeployedContractItem>,
    pub old_declared_contracts: Vec<Felt>,
    pub declared_classes: Vec<DeclaredClassItem>,
    pub nonces: HashMap<Felt, Felt>,
    pub replaced_classes: Vec<DeployedContractItem>,
}

impl From<mp_state_update::StateDiff> for StateDiff {
    fn from(state_diff: mp_state_update::StateDiff) -> Self {
        Self {
            storage_diffs: state_diff
                .storage_diffs
                .into_iter()
                .map(|mp_state_update::ContractStorageDiffItem { address, storage_entries }| (address, storage_entries))
                .collect(),
            deployed_contracts: state_diff.deployed_contracts,
            old_declared_contracts: state_diff.deprecated_declared_classes,
            declared_classes: state_diff.declared_classes,
            nonces: state_diff
                .nonces
                .into_iter()
                .map(|mp_state_update::NonceUpdate { contract_address, nonce }| (contract_address, nonce))
                .collect(),
            replaced_classes: state_diff
                .replaced_classes
                .into_iter()
                .map(|mp_state_update::ReplacedClassItem { contract_address, class_hash }| DeployedContractItem {
                    address: contract_address,
                    class_hash,
                })
                .collect(),
        }
    }
}
