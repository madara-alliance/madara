use std::collections::HashMap;

use mp_block::{FullBlock, PendingFullBlock};
use mp_state_update::{DeclaredClassItem, DeployedContractItem, StorageEntry};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use starknet_types_core::felt::Felt;

use crate::block::{FromGatewayError, ProviderBlock, ProviderBlockPending, ProviderBlockPendingMaybe};

#[derive(Debug, Clone, PartialEq, Serialize)] // no Deserialize because it's untagged
#[serde(untagged)]
pub enum ProviderStateUpdatePendingMaybe {
    NonPending(ProviderStateUpdate),
    Pending(ProviderStateUpdatePending),
}

impl ProviderStateUpdatePendingMaybe {
    pub fn non_pending(&self) -> Option<&ProviderStateUpdate> {
        match self {
            Self::NonPending(non_pending) => Some(non_pending),
            Self::Pending(_) => None,
        }
    }

    pub fn non_pending_owned(self) -> Option<ProviderStateUpdate> {
        match self {
            Self::NonPending(non_pending) => Some(non_pending),
            Self::Pending(_) => None,
        }
    }

    pub fn pending(&self) -> Option<&ProviderStateUpdatePending> {
        match self {
            Self::NonPending(_) => None,
            Self::Pending(pending) => Some(pending),
        }
    }

    pub fn pending_owned(self) -> Option<ProviderStateUpdatePending> {
        match self {
            Self::NonPending(_) => None,
            Self::Pending(pending) => Some(pending),
        }
    }

    pub fn state_diff(&self) -> &StateDiff {
        match self {
            Self::NonPending(non_pending) => &non_pending.state_diff,
            Self::Pending(pending) => &pending.state_diff,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderStateUpdate {
    pub block_hash: Felt,
    pub new_root: Felt,
    pub old_root: Felt,
    pub state_diff: StateDiff,
}

impl From<mp_state_update::StateUpdate> for ProviderStateUpdate {
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
pub struct ProviderStateUpdatePending {
    pub old_root: Felt,
    pub state_diff: StateDiff,
}

impl From<mp_state_update::PendingStateUpdate> for ProviderStateUpdatePending {
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

impl From<StateDiff> for mp_state_update::StateDiff {
    fn from(state_diff: StateDiff) -> Self {
        Self {
            storage_diffs: state_diff
                .storage_diffs
                .into_iter()
                .map(|(address, storage_entries)| mp_state_update::ContractStorageDiffItem { address, storage_entries })
                .collect(),
            deprecated_declared_classes: state_diff.old_declared_contracts,
            declared_classes: state_diff.declared_classes,
            deployed_contracts: state_diff.deployed_contracts,
            replaced_classes: state_diff
                .replaced_classes
                .into_iter()
                .map(|DeployedContractItem { address: contract_address, class_hash }| {
                    mp_state_update::ReplacedClassItem { contract_address, class_hash }
                })
                .collect(),
            nonces: state_diff
                .nonces
                .into_iter()
                .map(|(contract_address, nonce)| mp_state_update::NonceUpdate { contract_address, nonce })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)]
pub enum ProviderStateUpdateWithBlockPendingMaybe {
    NonPending(ProviderStateUpdateWithBlock),
    Pending(ProviderStateUpdateWithBlockPending),
}

impl ProviderStateUpdateWithBlockPendingMaybe {
    pub fn state_update(self) -> ProviderStateUpdatePendingMaybe {
        match self {
            Self::NonPending(non_pending) => ProviderStateUpdatePendingMaybe::NonPending(non_pending.state_update),
            Self::Pending(pending) => ProviderStateUpdatePendingMaybe::Pending(pending.state_update),
        }
    }

    pub fn block(self) -> ProviderBlockPendingMaybe {
        match self {
            Self::NonPending(non_pending) => ProviderBlockPendingMaybe::NonPending(non_pending.block),
            Self::Pending(pending) => ProviderBlockPendingMaybe::Pending(pending.block),
        }
    }

    pub fn as_update_and_block(self) -> (ProviderStateUpdatePendingMaybe, ProviderBlockPendingMaybe) {
        match self {
            Self::NonPending(ProviderStateUpdateWithBlock { state_update, block }) => (
                ProviderStateUpdatePendingMaybe::NonPending(state_update),
                ProviderBlockPendingMaybe::NonPending(block),
            ),
            Self::Pending(ProviderStateUpdateWithBlockPending { state_update, block }) => {
                (ProviderStateUpdatePendingMaybe::Pending(state_update), ProviderBlockPendingMaybe::Pending(block))
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[cfg_attr(test, derive(Eq))]
pub struct ProviderStateUpdateWithBlock {
    pub state_update: ProviderStateUpdate,
    pub block: ProviderBlock,
}

impl ProviderStateUpdateWithBlock {
    pub fn into_full_block(self) -> Result<FullBlock, FromGatewayError> {
        self.block.into_full_block(self.state_update.state_diff.into())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[cfg_attr(test, derive(Eq))]
pub struct ProviderStateUpdateWithBlockPending {
    pub state_update: ProviderStateUpdatePending,
    pub block: ProviderBlockPending,
}

impl ProviderStateUpdateWithBlockPending {
    pub fn into_full_block(self) -> Result<PendingFullBlock, FromGatewayError> {
        self.block.into_full_block(self.state_update.state_diff.into())
    }
}
