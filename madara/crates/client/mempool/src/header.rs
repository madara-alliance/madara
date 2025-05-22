use crate::L1DataProvider;
use mp_block::header::{BlockTimestamp, PendingHeader};
use mp_chain_config::ChainConfig;
use starknet_types_core::felt::Felt;

pub fn make_pending_header(
    parent_block_hash: Felt,
    chain_config: &ChainConfig,
    l1_info: &dyn L1DataProvider,
) -> PendingHeader {
    PendingHeader {
        parent_block_hash,
        sequencer_address: **chain_config.sequencer_address,
        block_timestamp: BlockTimestamp::now(),
        protocol_version: chain_config.latest_protocol_version,
        l1_gas_price: l1_info.get_gas_prices(),
        l1_da_mode: chain_config.l1_da_mode,
    }
}
