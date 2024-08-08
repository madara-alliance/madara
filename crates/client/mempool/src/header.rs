use crate::L1DataProvider;
use dp_block::{chain_config::ChainConfig, header::PendingHeader};
use starknet_core::types::Felt;
use std::time::SystemTime;

pub async fn make_pending_header(
    parent_block_hash: Felt,
    chain_config: &ChainConfig,
    l1_info: &dyn L1DataProvider,
) -> PendingHeader {
    PendingHeader {
        parent_block_hash,
        sequencer_address: **chain_config.sequencer_address,
        block_timestamp: SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("Current system time is before the UNIX epoch")
            .as_secs(),
        protocol_version: chain_config.latest_protocol_version,
        l1_gas_price: l1_info.get_gas_prices().await,
        l1_da_mode: l1_info.get_da_mode(),
    }
}
