use primitive_types::H160;
use starknet_types_core::felt::Felt;

use std::ops::Deref;

use starknet_api::block::BlockHash;
use starknet_api::core::{
    ChainId, ClassHash, CompiledClassHash, ContractAddress, EntryPointSelector, Nonce, PatriciaKey,
};
use starknet_api::transaction::{ContractAddressSalt, EventKey, TransactionHash};
use starknet_core::types::EthAddress;

pub trait ToFelt {
    fn to_felt(self) -> Felt;
}

impl ToFelt for EthAddress {
    fn to_felt(self) -> Felt {
        self.into()
    }
}

impl ToFelt for &EthAddress {
    fn to_felt(self) -> Felt {
        self.clone().to_felt()
    }
}

impl ToFelt for PatriciaKey {
    fn to_felt(self) -> Felt {
        *self.key()
    }
}

impl ToFelt for &PatriciaKey {
    fn to_felt(self) -> Felt {
        *self.deref()
    }
}

impl ToFelt for ContractAddress {
    fn to_felt(self) -> Felt {
        self.0.to_felt()
    }
}

impl ToFelt for &ContractAddress {
    fn to_felt(self) -> Felt {
        self.0.to_felt()
    }
}

impl ToFelt for H160 {
    fn to_felt(self) -> Felt {
        Felt::from_bytes_be_slice(&self.0)
    }
}

impl ToFelt for &ChainId {
    fn to_felt(self) -> Felt {
        let bytes: &[u8] = match self {
            ChainId::Mainnet => b"SN_MAIN",
            ChainId::Sepolia => b"SN_SEPOLIA",
            ChainId::IntegrationSepolia => b"SN_INTEGRATION_SEPOLIA",
            ChainId::Other(o) => o.as_bytes(),
        };
        Felt::from_bytes_be_slice(bytes)
    }
}

macro_rules! impl_for_wrapper {
    ($arg:ty) => {
        impl ToFelt for $arg {
            fn to_felt(self) -> Felt {
                self.0
            }
        }

        impl ToFelt for &$arg {
            fn to_felt(self) -> Felt {
                self.0
            }
        }
    };
}

impl_for_wrapper!(BlockHash);
impl_for_wrapper!(ClassHash);
impl_for_wrapper!(TransactionHash);
impl_for_wrapper!(EventKey);
impl_for_wrapper!(Nonce);
impl_for_wrapper!(EntryPointSelector);
impl_for_wrapper!(CompiledClassHash);
impl_for_wrapper!(ContractAddressSalt);
