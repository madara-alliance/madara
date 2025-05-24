use crate::eth::StarknetCoreContract;
use crate::state_update::StateUpdate;
use alloy::primitives::I256;
use anyhow::bail;
use mp_convert::ToFelt;
pub fn convert_log_state_update(log_state_update: StarknetCoreContract::LogStateUpdate) -> anyhow::Result<StateUpdate> {
    let block_number = if log_state_update.blockNumber >= I256::ZERO {
        log_state_update.blockNumber.low_u64()
    } else {
        bail!("Block number is negative");
    };

    let global_root = log_state_update.globalRoot.to_felt();
    let block_hash = log_state_update.blockHash.to_felt();

    Ok(StateUpdate { block_number: Some(block_number), global_root, block_hash })
}

#[cfg(test)]
mod eth_client_conversion_tests {
    use super::*;
    use alloy::primitives::U256;
    use starknet_types_core::felt::Felt;

    #[test]
    fn convert_log_state_update_works() {
        let block_number: u64 = 12345;
        let block_hash: u128 = 2345;
        let global_root: u128 = 456;

        let expected = StateUpdate {
            block_number: Some(block_number),
            block_hash: Felt::from(block_hash),
            global_root: Felt::from(global_root),
        };

        let input = StarknetCoreContract::LogStateUpdate {
            blockNumber: I256::from_dec_str(block_number.to_string().as_str()).unwrap(),
            blockHash: U256::from_str_radix(block_hash.to_string().as_str(), 10).unwrap(),
            globalRoot: U256::from_str_radix(global_root.to_string().as_str(), 10).unwrap(),
        };

        let result = convert_log_state_update(input).unwrap();

        assert_eq!(expected, result, "log state update conversion not working");
    }
}
