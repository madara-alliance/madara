use crate::client::StarknetCoreContract;
use crate::config::L1StateUpdate;
use alloy::primitives::{I256, U256};
use anyhow::bail;
use starknet_api::hash::StarkFelt;
use starknet_types_core::felt::Felt;

pub const LOG_STATE_UPDTATE_TOPIC: &str = "0xd342ddf7a308dec111745b00315c14b7efb2bdae570a6856e088ed0c65a3576c";
pub fn convert_log_state_update(log_state_update: StarknetCoreContract::LogStateUpdate) -> anyhow::Result<L1StateUpdate> {
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
