use std::ops::Deref;

use starknet_api::block::BlockHash;
use starknet_api::core::{ClassHash, ContractAddress, Nonce, PatriciaKey};
use starknet_api::hash::StarkFelt;
use starknet_api::transaction::{EventKey, TransactionHash};
use starknet_core::types::EthAddress;
use starknet_types_core::felt::Felt;

pub trait ToStarkFelt {
    fn to_stark_felt(self) -> StarkFelt;
}

impl ToStarkFelt for StarkFelt {
    fn to_stark_felt(self) -> StarkFelt {
        self
    }
}

impl ToStarkFelt for &StarkFelt {
    fn to_stark_felt(self) -> StarkFelt {
        *self
    }
}

impl ToStarkFelt for Felt {
    fn to_stark_felt(self) -> StarkFelt {
        StarkFelt::new_unchecked(self.to_bytes_be())
    }
}

impl ToStarkFelt for &Felt {
    fn to_stark_felt(self) -> StarkFelt {
        (*self).to_stark_felt()
    }
}

impl ToStarkFelt for EthAddress {
    fn to_stark_felt(self) -> StarkFelt {
        let mut output = [0u8; 32];
        output[..20].copy_from_slice(self.as_bytes());
        StarkFelt::new_unchecked(output)
    }
}

impl ToStarkFelt for &EthAddress {
    fn to_stark_felt(self) -> StarkFelt {
        self.clone().to_stark_felt()
    }
}

impl ToStarkFelt for PatriciaKey {
    fn to_stark_felt(self) -> StarkFelt {
        self.key().to_stark_felt()
    }
}

impl ToStarkFelt for &PatriciaKey {
    fn to_stark_felt(self) -> StarkFelt {
        self.deref().to_stark_felt()
    }
}

macro_rules! impl_for_wrapper {
    ($arg:ty) => {
        impl ToStarkFelt for $arg {
            fn to_stark_felt(self) -> StarkFelt {
                self.0.to_stark_felt()
            }
        }

        impl ToStarkFelt for &$arg {
            fn to_stark_felt(self) -> StarkFelt {
                self.0.to_stark_felt()
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
