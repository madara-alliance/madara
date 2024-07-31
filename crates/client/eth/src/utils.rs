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

    #[test]
    fn test_convert_log_state_update() {
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

    #[test]
    fn test_u256_to_starkfelt() {
        let random: u128 = 12345678123456789;
        let starkfelt = StarkFelt::from(random);

        let random_u256: U256 = U256::from_str_radix(random.to_string().as_str(), 10).unwrap();
        let result = u256_to_starkfelt(random_u256).unwrap();

        assert_eq!(result, starkfelt, "u256 to StarkFelt not working");
    }

    #[test]
    fn test_trim_hash() {
        // here we are testing trim hash the same way it's being used in the code
        // and that it to convert StarkFelt to felt and then sending it to the trim_hash

        let random: u128 = 30000000000000;
        // 0x1b48eb57e000 is the hex value of the given random
        let starkfelt = StarkFelt::from(random);
        let trimmed = trim_hash(&starkfelt.to_felt());
        assert_eq!(trimmed, "0x1b48eb...57e000");

        let random: u128 = 12345678123456789;
        // 0x2bdc542f0f5915 is the hex value of the given random
        let starkfelt = StarkFelt::from(random);
        let trimmed = trim_hash(&starkfelt.to_felt());
        assert_eq!(trimmed, "0x2bdc54...0f5915");
    }
}
