use crate::error::SettlementClientError;
use crate::eth::StarknetCoreContract;
use crate::state_update::StateUpdate;
use alloy::primitives::{I256, U256};
use anyhow::bail;
use starknet_types_core::felt::Felt;

pub fn convert_log_state_update(log_state_update: StarknetCoreContract::LogStateUpdate) -> anyhow::Result<StateUpdate> {
    let block_number = if log_state_update.blockNumber >= I256::ZERO {
        log_state_update.blockNumber.low_u64()
    } else {
        bail!("Block number is negative");
    };

    let global_root = u256_to_felt(log_state_update.globalRoot)?;
    let block_hash = u256_to_felt(log_state_update.blockHash)?;

    Ok(StateUpdate { block_number, global_root, block_hash })
}

pub fn u256_to_felt(u256: U256) -> Result<Felt, SettlementClientError> {
    let bytes = u256.to_be_bytes();
    Ok(Felt::from_bytes_be(&bytes))
}

pub fn felt_to_u256(felt: Felt) -> U256 {
    U256::from_be_bytes(felt.to_bytes_be())
}

#[cfg(test)]
mod eth_client_conversion_tests {
    use super::*;
    use rstest::*;

    #[test]
    fn convert_log_state_update_works() {
        let block_number: u64 = 12345;
        let block_hash: u128 = 2345;
        let global_root: u128 = 456;

        let expected =
            StateUpdate { block_number, block_hash: Felt::from(block_hash), global_root: Felt::from(global_root) };

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
    fn u256_to_felt_works(#[case] input: &str, #[case] expected_bytes: [u8; 32]) {
        let u256_value = U256::from_str_radix(input, 10).expect("Failed to parse U256");
        let expected = Felt::from_bytes_be(&expected_bytes);

        let result = u256_to_felt(u256_value).expect("u256_to_felt failed");

        assert_eq!(result, expected, "u256_to_felt failed for input: {}", input);
    }
}
