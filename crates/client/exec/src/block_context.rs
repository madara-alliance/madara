use blockifier::{
    context::{BlockContext, ChainInfo, FeeTokenAddresses},
    state::cached_state::{CachedState, GlobalContractCache},
    versioned_constants::VersionedConstants,
};
use dc_db::{db_block_id::DbBlockId, DeoxysBackend};
use dp_block::{
    header::{L1DataAvailabilityMode, BLOCKIFIER_VERSIONED_CONSTANTS_0_13_0, BLOCKIFIER_VERSIONED_CONSTANTS_0_13_1},
    DeoxysMaybePendingBlockInfo, StarknetVersion,
};
use dp_convert::ToStarkFelt;
use starknet_api::block::{BlockNumber, BlockTimestamp};
use starknet_types_core::felt::Felt;

use crate::{blockifier_state_adapter::BlockifierStateAdapter, Error};

pub const ETH_TOKEN_ADDR: Felt =
    Felt::from_hex_unchecked("0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7");

pub const STRK_TOKEN_ADDR: Felt =
    Felt::from_hex_unchecked("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d");

pub struct ExecutionContext<'a> {
    pub(crate) block_context: BlockContext,
    pub(crate) db_id: DbBlockId,
    pub(crate) backend: &'a DeoxysBackend,
}

impl<'a> ExecutionContext<'a> {
    pub fn init_cached_state(&self) -> CachedState<BlockifierStateAdapter<'a>> {
        let on_top_of = match self.db_id {
            DbBlockId::Pending => Some(DbBlockId::Pending),
            DbBlockId::BlockN(block_n) => {
                // We exec on top of the previous block. None means we are executing genesis.
                block_n.checked_sub(1).map(DbBlockId::BlockN)
            }
        };

        CachedState::new(BlockifierStateAdapter::new(self.backend, on_top_of), GlobalContractCache::new(16))
    }

    pub fn new(backend: &'a DeoxysBackend, block_info: &DeoxysMaybePendingBlockInfo) -> Result<Self, Error> {
        let (db_id, protocol_version, block_number, block_timestamp, sequencer_address, l1_gas_price, l1_da_mode) =
            match block_info {
                DeoxysMaybePendingBlockInfo::Pending(block) => (
                    DbBlockId::Pending,
                    block.header.protocol_version,
                    backend.get_latest_block_n()?.map(|el| el + 1).unwrap_or(0), // when the block is pending, we use the latest block n + 1
                    block.header.block_timestamp,
                    block.header.sequencer_address,
                    block.header.l1_gas_price.clone(),
                    block.header.l1_da_mode,
                ),
                DeoxysMaybePendingBlockInfo::NotPending(block) => (
                    DbBlockId::BlockN(block.header.block_number),
                    block.header.protocol_version,
                    block.header.block_number,
                    block.header.block_timestamp,
                    block.header.sequencer_address,
                    block.header.l1_gas_price.clone(),
                    block.header.l1_da_mode,
                ),
            };

        // safe unwrap because address is always valid and static
        let fee_token_addresses: FeeTokenAddresses = FeeTokenAddresses {
            strk_fee_token_address: STRK_TOKEN_ADDR.to_stark_felt().try_into().unwrap(),
            eth_fee_token_address: ETH_TOKEN_ADDR.to_stark_felt().try_into().unwrap(),
        };
        let chain_id = starknet_api::core::ChainId(
            String::from_utf8(backend.chain_info()?.chain_id.to_bytes_be().to_vec())
                .expect("Failed to convert chain id to string"),
        );

        let versioned_constants = if protocol_version < StarknetVersion::STARKNET_VERSION_0_13_0 {
            return Err(Error::UnsupportedProtocolVersion);
        } else if protocol_version < StarknetVersion::STARKNET_VERSION_0_13_1 {
            &BLOCKIFIER_VERSIONED_CONSTANTS_0_13_0
        } else if protocol_version < StarknetVersion::STARKNET_VERSION_0_13_1_1 {
            &BLOCKIFIER_VERSIONED_CONSTANTS_0_13_1
        } else {
            VersionedConstants::latest_constants()
        };

        let chain_info = ChainInfo { chain_id, fee_token_addresses };

        let block_info = blockifier::block::BlockInfo {
            block_number: BlockNumber(block_number),
            block_timestamp: BlockTimestamp(block_timestamp),
            sequencer_address: sequencer_address.to_stark_felt().try_into().unwrap(),
            gas_prices: (&l1_gas_price).into(),
            // TODO: Verify if this is correct
            use_kzg_da: l1_da_mode == L1DataAvailabilityMode::Blob,
        };

        Ok(ExecutionContext {
            block_context: BlockContext::new_unchecked(&block_info, &chain_info, versioned_constants),
            db_id,
            backend,
        })
    }
}
