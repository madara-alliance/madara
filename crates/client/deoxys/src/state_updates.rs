use mp_felt::Felt252Wrapper;
use starknet_gateway::sequencer::models::state_update::{StateDiff, StorageDiff, DeployedContract, DeclaredContract};
use parity_scale_codec::{Encode, Decode, Input, Output, Error};
use starknet_ff::FieldElement;

#[derive(Debug)]
#[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode, parity_scale_codec::Decode))]
pub struct StarknetStateUpdate(pub starknet_gateway::sequencer::models::StateUpdate);

impl StarknetStateUpdate {
    // Assuming `starknet_gateway::sequencer::models::StateUpdate` has these fields
    // and they are public or have getter methods that return the necessary types.

    pub fn get_block_hash(&self) -> Option<&FieldElement> {
        self.0.block_hash.as_ref()
    }

    pub fn get_new_root(&self) -> Option<&FieldElement> {
        self.0.new_root.as_ref()
    }

    pub fn get_old_root(&self) -> &FieldElement {
        &self.0.old_root
    }

    pub fn get_state_diff(&self) -> &StateDiff {
        &self.0.state_diff
    }
}

pub struct StateUpdateWrapper {
    pub block_hash: Option<Felt252Wrapper>,
    pub new_root: Option<Felt252Wrapper>,
    pub old_root: Felt252Wrapper,
    pub state_diff: StateDiffWrapper,
}

pub struct StateDiffWrapper {
    pub storage_diffs: Vec<(Felt252Wrapper, Vec<StorageDiffWrapper>)>,
    pub deployed_contracts: Vec<DeployedContractWrapper>,
    pub old_declared_contracts: Vec<Felt252Wrapper>,
    pub declared_classes: Vec<DeclaredContractWrapper>,
    pub nonces: Vec<(Felt252Wrapper, Felt252Wrapper)>,
    pub replaced_classes: Vec<DeployedContractWrapper>,
}

pub struct StorageDiffWrapper {
    pub key: Felt252Wrapper,
    pub value: Felt252Wrapper,
}

pub struct DeployedContractWrapper {
    pub address: Felt252Wrapper,
    pub class_hash: Felt252Wrapper,
}

pub struct DeclaredContractWrapper {
    pub class_hash: Felt252Wrapper,
    pub compiled_class_hash: Felt252Wrapper,
}

impl From<StarknetStateUpdate> for StateUpdateWrapper {
    fn from(update: StarknetStateUpdate) -> Self {
        StateUpdateWrapper {
            block_hash: update.get_block_hash().cloned().map(Felt252Wrapper::from),
            new_root: update.get_new_root().cloned().map(Felt252Wrapper::from),
            old_root: Felt252Wrapper::from(*update.get_old_root()),
            state_diff: StateDiffWrapper::from(update.get_state_diff()),
        }
    }
}

impl From<&StateDiff> for StateDiffWrapper {
    fn from(diff: &StateDiff) -> Self {
        StateDiffWrapper {
            storage_diffs: diff.storage_diffs.iter()
                .map(|(key, diffs)| (Felt252Wrapper(*key), diffs.iter().map(StorageDiffWrapper::from).collect()))
                .collect(),
            deployed_contracts: diff.deployed_contracts.iter().map(DeployedContractWrapper::from).collect(),
            old_declared_contracts: diff.old_declared_contracts.iter().map(|&hash| Felt252Wrapper(hash)).collect(),
            declared_classes: diff.declared_classes.iter().map(DeclaredContractWrapper::from).collect(),
            nonces: diff.nonces.iter()
                .map(|(&key, &value)| (Felt252Wrapper(key), Felt252Wrapper(value)))
                .collect(),
            replaced_classes: diff.replaced_classes.iter().map(DeployedContractWrapper::from).collect(),
        }
    }
}

impl From<&StorageDiff> for StorageDiffWrapper {
    fn from(diff: &StorageDiff) -> Self {
        StorageDiffWrapper {
            key: Felt252Wrapper(diff.key),
            value: Felt252Wrapper(diff.value),
        }
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

// Encode and Decode for `StorageDiffWrapper`
impl Encode for StorageDiffWrapper {
    fn size_hint(&self) -> usize {
        self.key.size_hint() + self.value.size_hint()
    }

    fn encode_to<T: Output + ?Sized>(&self, dest: &mut T) {
        self.key.encode_to(dest);
        self.value.encode_to(dest);
    }
}

impl Decode for StorageDiffWrapper {
    fn decode<I: Input>(input: &mut I) -> Result<Self, Error> {
        Ok(StorageDiffWrapper {
            key: Felt252Wrapper::decode(input)?,
            value: Felt252Wrapper::decode(input)?,
        })
    }
}

// Encode and Decode for `DeployedContractWrapper`
impl Encode for DeployedContractWrapper {
    fn size_hint(&self) -> usize {
        self.address.size_hint() + self.class_hash.size_hint()
    }

    fn encode_to<T: Output + ?Sized>(&self, dest: &mut T) {
        self.address.encode_to(dest);
        self.class_hash.encode_to(dest);
    }
}

impl Decode for DeployedContractWrapper {
    fn decode<I: Input>(input: &mut I) -> Result<Self, Error> {
        Ok(DeployedContractWrapper {
            address: Felt252Wrapper::decode(input)?,
            class_hash: Felt252Wrapper::decode(input)?,
        })
    }
}

// Encode and Decode for `DeclaredContractWrapper`
impl Encode for DeclaredContractWrapper {
    fn size_hint(&self) -> usize {
        self.class_hash.size_hint() + self.compiled_class_hash.size_hint()
    }

    fn encode_to<T: Output + ?Sized>(&self, dest: &mut T) {
        self.class_hash.encode_to(dest);
        self.compiled_class_hash.encode_to(dest);
    }
}

impl Decode for DeclaredContractWrapper {
    fn decode<I: Input>(input: &mut I) -> Result<Self, Error> {
        Ok(DeclaredContractWrapper {
            class_hash: Felt252Wrapper::decode(input)?,
            compiled_class_hash: Felt252Wrapper::decode(input)?,
        })
    }
}

// Encode and Decode for `StateDiffWrapper`
impl Encode for StateDiffWrapper {
    fn size_hint(&self) -> usize {
        // Implement size hint calculation based on the new Vec fields.
        0 // Replace this with the actual calculation.
    }

    fn encode_to<T: Output + ?Sized>(&self, dest: &mut T) {
        // Encode each of the Vec fields.
        self.storage_diffs.encode_to(dest);
        self.deployed_contracts.encode_to(dest);
        self.old_declared_contracts.encode_to(dest);
        self.declared_classes.encode_to(dest);
        self.nonces.encode_to(dest);
        self.replaced_classes.encode_to(dest);
    }
}

impl Decode for StateDiffWrapper {
    fn decode<I: Input>(input: &mut I) -> Result<Self, Error> {
        Ok(StateDiffWrapper {
            storage_diffs: Vec::<(Felt252Wrapper, Vec<StorageDiffWrapper>)>::decode(input)?,
            deployed_contracts: Vec::<DeployedContractWrapper>::decode(input)?,
            old_declared_contracts: Vec::<Felt252Wrapper>::decode(input)?,
            declared_classes: Vec::<DeclaredContractWrapper>::decode(input)?,
            nonces: Vec::<(Felt252Wrapper, Felt252Wrapper)>::decode(input)?,
            replaced_classes: Vec::<DeployedContractWrapper>::decode(input)?,
        })
    }
}


// Encode and Decode for `StateUpdateWrapper`
impl Encode for StateUpdateWrapper {
    fn size_hint(&self) -> usize {
        self.block_hash.size_hint() + self.new_root.size_hint() + self.old_root.size_hint() + self.state_diff.size_hint()
    }

    fn encode_to<T: Output + ?Sized>(&self, dest: &mut T) {
        self.block_hash.encode_to(dest);
        self.new_root.encode_to(dest);
        self.old_root.encode_to(dest);
        self.state_diff.encode_to(dest);
    }
}

impl Decode for StateUpdateWrapper {
    fn decode<I: Input>(input: &mut I) -> Result<Self, Error> {
        Ok(StateUpdateWrapper {
            block_hash: Option::<Felt252Wrapper>::decode(input)?,
            new_root: Option::<Felt252Wrapper>::decode(input)?,
            old_root: Felt252Wrapper::decode(input)?,
            state_diff: StateDiffWrapper::decode(input)?,
        })
    }
}
