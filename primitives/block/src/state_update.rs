#![cfg_attr(not(feature = "std"), no_std)]
// TODO: Find a better place than the block primitive crate to store this.
use mp_felt::Felt252Wrapper;

#[doc(hidden)]
pub extern crate alloc;
use alloc::vec::Vec;

#[derive(Debug)]
#[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode, parity_scale_codec::Decode))]
pub struct StateUpdateWrapper {
    pub block_hash: Option<Felt252Wrapper>,
    pub new_root: Option<Felt252Wrapper>,
    pub old_root: Felt252Wrapper,
    pub state_diff: StateDiffWrapper,
}

#[derive(Debug)]
#[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode, parity_scale_codec::Decode))]
pub struct StateDiffWrapper {
    pub storage_diffs: Vec<(Felt252Wrapper, Vec<StorageDiffWrapper>)>,
    pub deployed_contracts: Vec<DeployedContractWrapper>,
    pub old_declared_contracts: Vec<Felt252Wrapper>,
    pub declared_classes: Vec<DeclaredContractWrapper>,
    pub nonces: Vec<(Felt252Wrapper, Felt252Wrapper)>,
    pub replaced_classes: Vec<DeployedContractWrapper>,
}

#[derive(Debug)]
#[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode, parity_scale_codec::Decode))]
pub struct StorageDiffWrapper {
    pub key: Felt252Wrapper,
    pub value: Felt252Wrapper,
}

#[derive(Debug)]
#[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode, parity_scale_codec::Decode))]
pub struct DeployedContractWrapper {
    pub address: Felt252Wrapper,
    pub class_hash: Felt252Wrapper,
}

#[derive(Debug)]
#[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode, parity_scale_codec::Decode))]
pub struct DeclaredContractWrapper {
    pub class_hash: Felt252Wrapper,
    pub compiled_class_hash: Felt252Wrapper,
}

/// module `starknet-provider` uses `std` and as result cannot be included in the
/// substrate runtime. This ensures conversions using `starknet-provider` are
/// only available in `std` environments.
#[cfg(feature = "std")]
pub mod convert {
    use starknet_providers::sequencer::models::state_update::{
        DeclaredContract, DeployedContract, StateDiff, StorageDiff,
    };
    use starknet_providers::sequencer::models::StateUpdate;

    use super::*;

    impl From<starknet_providers::sequencer::models::StateUpdate> for StateUpdateWrapper {
        fn from(update: StateUpdate) -> Self {
            StateUpdateWrapper {
                block_hash: update.block_hash.as_ref().cloned().map(Felt252Wrapper::from),
                new_root: update.new_root.as_ref().cloned().map(Felt252Wrapper::from),
                old_root: Felt252Wrapper::from(update.old_root),
                state_diff: StateDiffWrapper::from(&update.state_diff),
            }
        }
    }

    impl From<&StateDiff> for StateDiffWrapper {
        fn from(diff: &StateDiff) -> Self {
            StateDiffWrapper {
                storage_diffs: diff
                    .storage_diffs
                    .iter()
                    .map(|(key, diffs)| (Felt252Wrapper(*key), diffs.iter().map(StorageDiffWrapper::from).collect()))
                    .collect(),
                deployed_contracts: diff.deployed_contracts.iter().map(DeployedContractWrapper::from).collect(),
                old_declared_contracts: diff.old_declared_contracts.iter().map(|&hash| Felt252Wrapper(hash)).collect(),
                declared_classes: diff.declared_classes.iter().map(DeclaredContractWrapper::from).collect(),
                nonces: diff.nonces.iter().map(|(&key, &value)| (Felt252Wrapper(key), Felt252Wrapper(value))).collect(),
                replaced_classes: diff.replaced_classes.iter().map(DeployedContractWrapper::from).collect(),
            }
        }
    }

    impl From<&StorageDiff> for StorageDiffWrapper {
        fn from(diff: &StorageDiff) -> Self {
            StorageDiffWrapper { key: Felt252Wrapper(diff.key), value: Felt252Wrapper(diff.value) }
        }
    }

    impl From<&DeployedContract> for DeployedContractWrapper {
        fn from(contract: &DeployedContract) -> Self {
            DeployedContractWrapper {
                address: Felt252Wrapper(contract.address),
                class_hash: Felt252Wrapper(contract.class_hash),
            }
        }
    }

    impl From<&DeclaredContract> for DeclaredContractWrapper {
        fn from(contract: &DeclaredContract) -> Self {
            DeclaredContractWrapper {
                class_hash: Felt252Wrapper(contract.class_hash),
                compiled_class_hash: Felt252Wrapper(contract.compiled_class_hash),
            }
        }
    }
}
