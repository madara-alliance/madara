use std::ops::Deref;

use starknet_api::block::BlockHash;
use starknet_api::core::{ClassHash, ContractAddress, Nonce, PatriciaKey};
use starknet_api::hash::StarkFelt;
use starknet_api::transaction::{EventKey, TransactionHash};
use starknet_core::types::EthAddress;
use starknet_types_core::felt::Felt;

pub trait FeltWrapper {
    fn into_stark_felt(self) -> StarkFelt;
}

impl FeltWrapper for StarkFelt {
    fn into_stark_felt(self) -> StarkFelt {
        self
    }
}

impl FeltWrapper for &StarkFelt {
    fn into_stark_felt(self) -> StarkFelt {
        *self
    }
}

impl FeltWrapper for Felt {
    fn into_stark_felt(self) -> StarkFelt {
        StarkFelt::new_unchecked(self.to_bytes_be())
    }
}

impl FeltWrapper for &Felt {
    fn into_stark_felt(self) -> StarkFelt {
        (*self).into_stark_felt()
    }
}

impl FeltWrapper for EthAddress {
    fn into_stark_felt(self) -> StarkFelt {
        let mut output = [0u8; 32];
        output[..20].copy_from_slice(self.as_bytes());
        StarkFelt::new_unchecked(output)
    }
}

impl FeltWrapper for &EthAddress {
    fn into_stark_felt(self) -> StarkFelt {
        self.clone().into_stark_felt()
    }
}

impl FeltWrapper for PatriciaKey {
    fn into_stark_felt(self) -> StarkFelt {
        self.key().into_stark_felt()
    }
}

impl FeltWrapper for &PatriciaKey {
    fn into_stark_felt(self) -> StarkFelt {
        self.deref().into_stark_felt()
    }
}

macro_rules! impl_for_wrapper {
    ($arg:ty) => {
        impl FeltWrapper for $arg {
            fn into_stark_felt(self) -> StarkFelt {
                self.0.into_stark_felt()
            }
        }

        impl FeltWrapper for &$arg {
            fn into_stark_felt(self) -> StarkFelt {
                self.0.into_stark_felt()
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
