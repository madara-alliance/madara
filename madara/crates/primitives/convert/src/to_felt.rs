use alloy::primitives::U256;
use core::fmt;
use primitive_types::H160;
use starknet_api::block::BlockHash;
use starknet_api::core::{
    ChainId, ClassHash, CompiledClassHash, ContractAddress, EntryPointSelector, L1Address, Nonce, PatriciaKey,
};
use starknet_api::transaction::fields::ContractAddressSalt;
use starknet_api::transaction::{EventKey, TransactionHash};
use starknet_core::types::EthAddress;
use std::ops::Deref;

pub use starknet_types_core::felt::Felt;

pub trait ToFelt {
    fn to_felt(self) -> Felt;
}

impl ToFelt for L1Address {
    fn to_felt(self) -> Felt {
        self.into()
    }
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

impl ToFelt for U256 {
    fn to_felt(self) -> Felt {
        Felt::from_bytes_be(&self.to_be_bytes())
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

pub trait FeltHexDisplay {
    /// Force-display this felt as hexadecimal when using the [`fmt::Display`] or [`fmt::Debug`] traits.
    fn hex_display(self) -> DisplayFeltAsHex;
}
impl<T: ToFelt> FeltHexDisplay for T {
    fn hex_display(self) -> DisplayFeltAsHex {
        DisplayFeltAsHex(self.to_felt())
    }
}
impl FeltHexDisplay for Felt {
    fn hex_display(self) -> DisplayFeltAsHex {
        DisplayFeltAsHex(self)
    }
}

pub struct DisplayFeltAsHex(pub Felt);
impl fmt::Display for DisplayFeltAsHex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#x}", self.0)
    }
}
impl fmt::Debug for DisplayFeltAsHex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::*;
    use starknet_core::types::EthAddress;

    #[test]
    fn test_eth_address_to_felt() {
        let hex = "0x123456789abcdef0123456789abcdef012345678";
        let eth_address = EthAddress::from_hex(hex).unwrap();
        assert_eq!((&eth_address).to_felt(), Felt::from_hex_unchecked(hex));
        assert_eq!(eth_address.to_felt(), Felt::from_hex_unchecked(hex));
    }

    #[test]
    fn test_patricia_key_to_felt() {
        let key: u128 = 0x123456789abcdef0123456789abcdef;
        let patricia_key: PatriciaKey = key.into();
        assert_eq!((&patricia_key).to_felt(), Felt::from(key));
        assert_eq!(patricia_key.to_felt(), Felt::from(key));
    }

    #[test]
    fn test_contract_address_to_felt() {
        let address: u128 = 0x123456789abcdef0123456789abcdef;
        let contract_address: ContractAddress = address.into();
        assert_eq!((&contract_address).to_felt(), Felt::from(address));
        assert_eq!(contract_address.to_felt(), Felt::from(address));
    }

    #[test]
    fn test_h160_to_felt() {
        let value: [u8; 20] = [
            0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x12, 0x34,
            0x56, 0x78,
        ];
        let h160 = H160::from_slice(&value);
        assert_eq!(h160.to_felt(), Felt::from_bytes_be_slice(&value));
    }

    #[test]
    fn test_chain_id_to_felt() {
        let main_chain_id = ChainId::Mainnet;
        let expected_chain_id = Felt::from_hex_unchecked("0x534e5f4d41494e");
        assert_eq!(main_chain_id.to_felt(), expected_chain_id);

        let sepolia_chain_id = ChainId::Sepolia;
        let expected_chain_id = Felt::from_hex_unchecked("0x534e5f5345504f4c4941");
        assert_eq!(sepolia_chain_id.to_felt(), expected_chain_id);

        let integration_sepolia_chain_id = ChainId::IntegrationSepolia;
        let expected_chain_id = Felt::from_hex_unchecked("0x534e5f494e544547524154494f4e5f5345504f4c4941");
        assert_eq!(integration_sepolia_chain_id.to_felt(), expected_chain_id);

        let other_chain_id = ChainId::Other("SN_OTHER".to_string());
        let expected_chain_id = Felt::from_hex_unchecked("0x534e5f4f54484552");
        assert_eq!(other_chain_id.to_felt(), expected_chain_id);
    }

    #[rstest]
    #[case(
        "12345678123456789",
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x2b, 0xdc, 0x54, 0x2f, 0x0f, 0x59, 0x15]
    )]
    #[case(
        "340282366920938463463374607431768211455",
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]
    )]
    #[case(
        "7237005577332262213973186563042994240829374041602535252466099000494570602495",
        [0x0f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]
    )]
    fn u256_to_felt_works(#[case] input: &str, #[case] expected_bytes: [u8; 32]) {
        let u256_value = U256::from_str_radix(input, 10).expect("Failed to parse U256");
        let expected = Felt::from_bytes_be(&expected_bytes);

        let result = u256_value.to_felt();

        assert_eq!(result, expected, "u256_to_felt failed for input: {}", input);
    }
}
