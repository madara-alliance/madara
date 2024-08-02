use crate::client::StarknetCoreContract;
use crate::state_update::L1StateUpdate;
use alloy::primitives::{I256, U256};
use anyhow::bail;
use starknet_api::hash::StarkFelt;
use starknet_types_core::felt::Felt;
pub fn convert_log_state_update(
    log_state_update: StarknetCoreContract::LogStateUpdate,
) -> anyhow::Result<L1StateUpdate> {
    let block_number = if log_state_update.blockNumber >= I256::ZERO {
        log_state_update.blockNumber.low_u64()
    } else {
        bail!("Block number is negative");
    };

    let global_root = u256_to_starkfelt(log_state_update.globalRoot)?;
    let block_hash = u256_to_starkfelt(log_state_update.blockHash)?;

    Ok(L1StateUpdate { block_number, global_root, block_hash })
}

pub fn u256_to_starkfelt(u256: U256) -> anyhow::Result<StarkFelt> {
    let binding = u256.to_be_bytes_vec();
    let bytes = binding.as_slice();
    let mut bytes_array = [0u8; 32];
    bytes_array.copy_from_slice(bytes);
    let starkfelt = StarkFelt::new(bytes_array)?;
    Ok(starkfelt)
}

pub fn trim_hash(hash: &Felt) -> String {
    let hash_str = format!("{:#x}", hash);
    let hash_len = hash_str.len();
    let prefix = &hash_str[..6 + 2];
    let suffix = &hash_str[hash_len - 6..];
    format!("{}...{}", prefix, suffix)
}

#[cfg(test)]
mod eth_client_conversion_tests {
    use super::*;
    use dp_convert::ToFelt;
    use rstest::*;

    #[test]
    fn convert_log_state_update_works() {
        let block_number: u64 = 12345;
        let block_hash: u128 = 2345;
        let global_root: u128 = 456;

        let expected = L1StateUpdate {
            block_number,
            block_hash: StarkFelt::from(block_hash),
            global_root: StarkFelt::from(global_root),
        };

        let input = StarknetCoreContract::LogStateUpdate {
            blockNumber: I256::from_dec_str(block_number.to_string().as_str()).unwrap(),
            blockHash: U256::from_str_radix(block_hash.to_string().as_str(), 10).unwrap(),
            globalRoot: U256::from_str_radix(global_root.to_string().as_str(), 10).unwrap(),
        };

        let result = convert_log_state_update(input).unwrap();

        assert_eq!(expected, result, "log state update conversion not working");
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
    fn u256_to_starkfelt_works(#[case] input: &str, #[case] expected_bytes: [u8; 32]) {
        let u256_value = U256::from_str_radix(input, 10).expect("Failed to parse U256");
        let expected = StarkFelt::new(expected_bytes).expect("Failed to create expected StarkFelt");

        let result = u256_to_starkfelt(u256_value).expect("u256_to_starkfelt failed");

        assert_eq!(result, expected, "u256_to_starkfelt failed for input: {}", input);
    }

    #[rstest]
    #[case(30000000000000, "0x1b48eb...57e000")]
    #[case(12345678123456789, "0x2bdc54...0f5915")]
    fn trim_hash_works(#[case] input: u128, #[case] expected: &str) {
        let starkfelt = StarkFelt::from(input);
        let trimmed = trim_hash(&starkfelt.to_felt());
        assert_eq!(trimmed, expected);
    }
}
