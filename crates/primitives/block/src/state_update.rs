#![cfg_attr(not(feature = "std"), no_std)]
// TODO: Find a better place than the block primitive crate to store this.
use mp_felt::Felt252Wrapper;
#[cfg(feature = "parity-scale-codec")]
use parity_scale_codec::{Decode, Encode, Error, Input, Output};

#[doc(hidden)]
pub extern crate alloc;

use alloc::vec::Vec;

#[derive(Debug)]
pub struct StateUpdateWrapper {
    pub block_hash: Option<Felt252Wrapper>,
    pub new_root: Option<Felt252Wrapper>,
    pub old_root: Felt252Wrapper,
    pub state_diff: StateDiffWrapper,
}

#[derive(Debug)]
pub struct StateDiffWrapper {
    pub storage_diffs: Vec<(Felt252Wrapper, Vec<StorageDiffWrapper>)>,
    pub deployed_contracts: Vec<DeployedContractWrapper>,
    pub old_declared_contracts: Vec<Felt252Wrapper>,
    pub declared_classes: Vec<DeclaredContractWrapper>,
    pub nonces: Vec<(Felt252Wrapper, Felt252Wrapper)>,
    pub replaced_classes: Vec<DeployedContractWrapper>,
}

#[derive(Debug)]
pub struct StorageDiffWrapper {
    pub key: Felt252Wrapper,
    pub value: Felt252Wrapper,
}

#[derive(Debug)]
pub struct DeployedContractWrapper {
    pub address: Felt252Wrapper,
    pub class_hash: Felt252Wrapper,
}

#[derive(Debug)]
pub struct DeclaredContractWrapper {
    pub class_hash: Felt252Wrapper,
    pub compiled_class_hash: Felt252Wrapper,
}

// Encode and Decode for `StorageDiffWrapper`
#[cfg(feature = "parity-scale-codec")]
impl Encode for StorageDiffWrapper {
    fn size_hint(&self) -> usize {
        self.key.size_hint() + self.value.size_hint()
    }

    fn encode_to<T: Output + ?Sized>(&self, dest: &mut T) {
        self.key.encode_to(dest);
        self.value.encode_to(dest);
    }
}

#[cfg(feature = "parity-scale-codec")]
impl Decode for StorageDiffWrapper {
    fn decode<I: Input>(input: &mut I) -> Result<Self, Error> {
        Ok(StorageDiffWrapper {
            key: Felt252Wrapper::decode(input)?,
            value: Felt252Wrapper::decode(input)?,
        })
    }
}

// Encode and Decode for `DeployedContractWrapper`
#[cfg(feature = "parity-scale-codec")]
impl Encode for DeployedContractWrapper {
    fn size_hint(&self) -> usize {
        self.address.size_hint() + self.class_hash.size_hint()
    }

    fn encode_to<T: Output + ?Sized>(&self, dest: &mut T) {
        self.address.encode_to(dest);
        self.class_hash.encode_to(dest);
    }
}

#[cfg(feature = "parity-scale-codec")]
impl Decode for DeployedContractWrapper {
    fn decode<I: Input>(input: &mut I) -> Result<Self, Error> {
        Ok(DeployedContractWrapper {
            address: Felt252Wrapper::decode(input)?,
            class_hash: Felt252Wrapper::decode(input)?,
        })
    }
}

// Encode and Decode for `DeclaredContractWrapper`
#[cfg(feature = "parity-scale-codec")]
impl Encode for DeclaredContractWrapper {
    fn size_hint(&self) -> usize {
        self.class_hash.size_hint() + self.compiled_class_hash.size_hint()
    }

    fn encode_to<T: Output + ?Sized>(&self, dest: &mut T) {
        self.class_hash.encode_to(dest);
        self.compiled_class_hash.encode_to(dest);
    }
}

#[cfg(feature = "parity-scale-codec")]
impl Decode for DeclaredContractWrapper {
    fn decode<I: Input>(input: &mut I) -> Result<Self, Error> {
        Ok(DeclaredContractWrapper {
            class_hash: Felt252Wrapper::decode(input)?,
            compiled_class_hash: Felt252Wrapper::decode(input)?,
        })
    }
}

// Encode and Decode for `StateDiffWrapper`
#[cfg(feature = "parity-scale-codec")]
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

#[cfg(feature = "parity-scale-codec")]
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
#[cfg(feature = "parity-scale-codec")]
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

#[cfg(feature = "parity-scale-codec")]
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