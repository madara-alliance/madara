use starknet_api::block::BlockHash;
use starknet_api::core::{ClassHash, ContractAddress, Nonce, PatriciaKey};
use starknet_api::hash::StarkFelt;
use starknet_api::transaction::{EventKey, TransactionHash};
use starknet_core::types::EthAddress;
use starknet_ff::FieldElement;

use crate::Felt252Wrapper;

pub trait FeltWrapper {
    fn into_stark_felt(self) -> StarkFelt;
    fn into_field_element(self) -> FieldElement;
}

impl FeltWrapper for FieldElement {
    fn into_stark_felt(self) -> StarkFelt {
        Felt252Wrapper(self).into()
    }
    fn into_field_element(self) -> FieldElement {
        self
    }
}

impl FeltWrapper for &FieldElement {
    fn into_stark_felt(self) -> StarkFelt {
        Felt252Wrapper(*self).into()
    }
    fn into_field_element(self) -> FieldElement {
        *self
    }
}

impl FeltWrapper for StarkFelt {
    fn into_stark_felt(self) -> StarkFelt {
        self
    }
    fn into_field_element(self) -> FieldElement {
        Felt252Wrapper::from(self).into()
    }
}

impl FeltWrapper for &StarkFelt {
    fn into_stark_felt(self) -> StarkFelt {
        *self
    }
    fn into_field_element(self) -> FieldElement {
        Felt252Wrapper::from(*self).into()
    }
}

impl FeltWrapper for Felt252Wrapper {
    fn into_stark_felt(self) -> StarkFelt {
        self.into()
    }
    fn into_field_element(self) -> FieldElement {
        self.into()
    }
}

impl FeltWrapper for &Felt252Wrapper {
    fn into_stark_felt(self) -> StarkFelt {
        (*self).into_stark_felt()
    }
    fn into_field_element(self) -> FieldElement {
        (*self).into_field_element()
    }
}

impl FeltWrapper for EthAddress {
    fn into_stark_felt(self) -> StarkFelt {
        Felt252Wrapper::from(self).into()
    }
    fn into_field_element(self) -> FieldElement {
        Felt252Wrapper::from(self).into()
    }
}

impl FeltWrapper for &EthAddress {
    fn into_stark_felt(self) -> StarkFelt {
        self.clone().into_stark_felt()
    }
    fn into_field_element(self) -> FieldElement {
        self.clone().into_field_element()
    }
}

macro_rules! impl_for_wrapper {
    ($arg:ty) => {
        impl FeltWrapper for $arg {
            fn into_stark_felt(self) -> StarkFelt {
                self.0.into_stark_felt()
            }
            fn into_field_element(self) -> FieldElement {
                self.0.into_field_element()
            }
        }

        impl FeltWrapper for &$arg {
            fn into_stark_felt(self) -> StarkFelt {
                self.0.into_stark_felt()
            }
            fn into_field_element(self) -> FieldElement {
                self.0.into_field_element()
            }
        }
    };
}

impl_for_wrapper!(PatriciaKey);
impl_for_wrapper!(BlockHash);
impl_for_wrapper!(ClassHash);
impl_for_wrapper!(TransactionHash);
impl_for_wrapper!(ContractAddress);
impl_for_wrapper!(EventKey);
impl_for_wrapper!(Nonce);
