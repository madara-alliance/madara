use crate::{blockifier_state_adapter::BlockifierStateAdapter, Error};
use blockifier::{
    blockifier::{
        config::TransactionExecutorConfig, stateful_validator::StatefulValidator,
        transaction_executor::TransactionExecutor,
    },
    context::{BlockContext, ChainInfo, FeeTokenAddresses},
    state::cached_state::CachedState,
};
use mc_db::{db_block_id::DbBlockId, MadaraBackend};
use mp_block::{header::L1DataAvailabilityMode, MadaraMaybePendingBlockInfo};
use starknet_api::block::{BlockNumber, BlockTimestamp};
use std::sync::Arc;

pub struct ExecutionContext {
    pub(crate) backend: Arc<MadaraBackend>,
    pub(crate) block_context: BlockContext,
    pub(crate) db_id: DbBlockId,
}

impl ExecutionContext {
    pub fn tx_executor(&self) -> TransactionExecutor<BlockifierStateAdapter> {
        TransactionExecutor::new(
            self.init_cached_state(),
            self.block_context.clone(),
            // No concurrency yet.
            TransactionExecutorConfig { concurrency_config: Default::default() },
        )
    }

    pub fn tx_validator(&self) -> StatefulValidator<BlockifierStateAdapter> {
        StatefulValidator::create(self.init_cached_state(), self.block_context.clone())
    }

    pub fn init_cached_state(&self) -> CachedState<BlockifierStateAdapter> {
        let on_top_of = match self.db_id {
            DbBlockId::Pending => Some(DbBlockId::Pending),
            DbBlockId::BlockN(block_n) => {
                // We exec on top of the previous block. None means we are executing genesis.
                block_n.checked_sub(1).map(DbBlockId::BlockN)
            }
        };

        log::debug!(
            "Init cached state on top of {:?}, block number {:?}",
            on_top_of,
            self.block_context.block_info().block_number.0
        );

        CachedState::new(BlockifierStateAdapter::new(
            Arc::clone(&self.backend),
            self.block_context.block_info().block_number.0,
            on_top_of,
        ))
    }

    /// Create an execution context for executing transactions **within** that block.
    pub fn new_in_block(backend: Arc<MadaraBackend>, block_info: &MadaraMaybePendingBlockInfo) -> Result<Self, Error> {
        let (db_id, protocol_version, block_number, block_timestamp, sequencer_address, l1_gas_price, l1_da_mode) =
            match block_info {
                MadaraMaybePendingBlockInfo::Pending(block) => (
                    DbBlockId::Pending,
                    block.header.protocol_version,
                    // when the block is pending, we use the latest block n + 1
                    // if there is no latest block, the pending block is actually the genesis block
                    backend.get_latest_block_n()?.map(|el| el + 1).unwrap_or(0),
                    block.header.block_timestamp,
                    block.header.sequencer_address,
                    block.header.l1_gas_price.clone(),
                    block.header.l1_da_mode,
                ),
                MadaraMaybePendingBlockInfo::NotPending(block) => (
                    DbBlockId::BlockN(block.header.block_number),
                    block.header.protocol_version,
                    block.header.block_number,
                    block.header.block_timestamp,
                    block.header.sequencer_address,
                    block.header.l1_gas_price.clone(),
                    block.header.l1_da_mode,
                ),
            };

        let versioned_constants = backend.chain_config().exec_constants_by_protocol_version(protocol_version)?;
        let chain_info = ChainInfo {
            chain_id: backend.chain_config().chain_id.clone(),
            fee_token_addresses: FeeTokenAddresses {
                strk_fee_token_address: backend.chain_config().native_fee_token_address,
                eth_fee_token_address: backend.chain_config().parent_fee_token_address,
            },
        };
        let block_info = blockifier::blockifier::block::BlockInfo {
            block_number: BlockNumber(block_number),
            block_timestamp: BlockTimestamp(block_timestamp),
            sequencer_address: sequencer_address
                .try_into()
                .map_err(|_| Error::InvalidSequencerAddress(sequencer_address))?,
            gas_prices: (&l1_gas_price).into(),
            use_kzg_da: l1_da_mode == L1DataAvailabilityMode::Blob,
        };

        Ok(ExecutionContext {
            block_context: BlockContext::new(
                block_info,
                chain_info,
                versioned_constants,
                backend.chain_config().bouncer_config.clone(),
            ),
            db_id,
            backend,
        })
    }
}
