use starknet_types_core::felt::Felt;

use std::ops::Deref;

use starknet_api::block::BlockHash;
use starknet_api::core::{ClassHash, CompiledClassHash, ContractAddress, EntryPointSelector, Nonce, PatriciaKey};
use starknet_api::hash::StarkFelt;
use starknet_api::transaction::{ContractAddressSalt, EventKey, TransactionHash};
use starknet_core::types::EthAddress;

pub trait CoreFelt {
    fn into_core_felt(self) -> Felt;
}

impl CoreFelt for StarkFelt {
    fn into_core_felt(self) -> Felt {
        Felt::from_bytes_be_slice(self.bytes())
    }
}

impl CoreFelt for &StarkFelt {
    fn into_core_felt(self) -> Felt {
        Felt::from_bytes_be_slice(self.bytes())
    }
}

impl CoreFelt for EthAddress {
    fn into_core_felt(self) -> Felt {
        self.into()
    }
}

impl CoreFelt for &EthAddress {
    fn into_core_felt(self) -> Felt {
        self.clone().into_core_felt()
    }
}

impl CoreFelt for PatriciaKey {
    fn into_core_felt(self) -> Felt {
        self.key().into_core_felt()
    }
}

impl CoreFelt for &PatriciaKey {
    fn into_core_felt(self) -> Felt {
        self.deref().into_core_felt()
    }
}

macro_rules! impl_for_wrapper {
    ($arg:ty) => {
        impl CoreFelt for $arg {
            fn into_core_felt(self) -> Felt {
                self.0.into_core_felt()
            }
        }

        impl CoreFelt for &$arg {
            fn into_core_felt(self) -> Felt {
                self.0.into_core_felt()
            }
        }
    };
}

impl_for_wrapper!(BlockHash);
impl_for_wrapper!(ClassHash);
impl_for_wrapper!(TransactionHash);
impl_for_wrapper!(ContractAddress);
impl_for_wrapper!(EventKey);
impl_for_wrapper!(Nonce);
impl_for_wrapper!(EntryPointSelector);
impl_for_wrapper!(CompiledClassHash);
impl_for_wrapper!(ContractAddressSalt);
