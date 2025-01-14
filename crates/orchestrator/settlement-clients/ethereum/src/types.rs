use alloy::network::{Ethereum, EthereumWallet};
use alloy::providers::fillers::{ChainIdFiller, FillProvider, GasFiller, JoinFill, NonceFiller, WalletFiller};
use alloy::providers::{Identity, RootProvider};
use alloy::transports::http::{Client, Http};
use alloy_primitives::U256;

pub type LocalWalletSignerMiddleware = FillProvider<
    JoinFill<
        JoinFill<JoinFill<JoinFill<Identity, GasFiller>, NonceFiller>, ChainIdFiller>,
        WalletFiller<EthereumWallet>,
    >,
    RootProvider<Http<Client>>,
    Http<Client>,
    Ethereum,
>;

pub type EthHttpProvider = FillProvider<
    JoinFill<
        JoinFill<JoinFill<JoinFill<Identity, GasFiller>, NonceFiller>, ChainIdFiller>,
        WalletFiller<EthereumWallet>,
    >,
    RootProvider<Http<Client>>,
    Http<Client>,
    Ethereum,
>;

pub fn convert_stark_bigint_to_u256(y_low: u128, y_high: u128) -> U256 {
    let y_high_u256 = U256::from(y_high);
    let y_low_u256 = U256::from(y_low);
    let shifted = y_high_u256 << 128;
    shifted + y_low_u256
}

pub fn bytes_be_to_u128(bytes: &[u8; 32]) -> u128 {
    let mut result: u128 = 0;

    // Since u128 is 16 bytes, we'll use the last 16 bytes of the input array
    // Starting from index 16 to get the least significant bytes
    for &byte in bytes[16..32].iter() {
        result = (result << 8) | byte as u128;
    }

    result
}

#[cfg(test)]
mod tests {
    use rstest::*;

    use super::*;

    #[rstest]
    #[case(0, 0, U256::from(0))]
    #[case(12345, 0, U256::from(12345u128))]
    #[case(0, 67890, U256::from(67890u128) << 128)]
    #[case(u128::MAX, 1, (U256::from(1u128) << 128) + U256::from(u128::MAX))]
    #[case(u128::MAX, u128::MAX, U256::MAX)]
    fn test_convert_stark_bigint_to_u256(#[case] y_low: u128, #[case] y_high: u128, #[case] expected: U256) {
        let result = convert_stark_bigint_to_u256(y_low, y_high);
        assert_eq!(result, expected);
    }

    // Helper function to create test bytes
    fn create_test_bytes(values: &[(usize, u8)]) -> [u8; 32] {
        let mut bytes = [0u8; 32];
        for &(index, value) in values {
            bytes[index] = value;
        }
        bytes
    }

    #[rstest]
    #[case::all_zeros(&[], 0u128)]
    #[case::last_byte_is_1(&[(31, 1)], 1u128)]
    #[case::last_byte_is_255(&[(31, 255)], 255u128)]
    #[case::last_two_bytes(&[(30, 1), (31, 255)], 511u128)]
    #[case(&[(20, 1), (25, 2), (31, 3)], (1u128 << 88) | (2u128 << 48) | 3u128)]
    #[case(&[(16, 1)], 1u128 << 120)]
    #[case(&[(31, 1), (30, 1), (29, 1)], 0x010101u128)]
    fn test_bytes_to_u128(#[case] byte_values: &[(usize, u8)], #[case] expected: u128) {
        let bytes = create_test_bytes(byte_values);
        let result = bytes_be_to_u128(&bytes);
        assert_eq!(result, expected);
    }

    #[rstest]
    fn test_bytes_to_u128_ignores_first_16_bytes() {
        let mut bytes = [255u8; 32];
        bytes[16..32].fill(0);
        let result = bytes_be_to_u128(&bytes);
        assert_eq!(result, 0);
    }

    #[rstest]
    fn test_bytes_to_u128_max_value() {
        let mut bytes = [0u8; 32];
        bytes[16..32].fill(255);
        let result = bytes_be_to_u128(&bytes);
        assert_eq!(result, u128::MAX);
    }
}
