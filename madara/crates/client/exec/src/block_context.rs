use crate::{blockifier_state_adapter::BlockifierStateAdapter, Error, LayeredStateAdaptor};
use blockifier::{
    blockifier::{
        block::BlockInfo, config::TransactionExecutorConfig, stateful_validator::StatefulValidator,
        transaction_executor::TransactionExecutor,
    },
    context::BlockContext,
    state::cached_state::CachedState,
};
use mc_db::{db_block_id::DbBlockId, MadaraBackend};
use mp_block::MadaraMaybePendingBlockInfo;

use mp_chain_config::L1DataAvailabilityMode;
use starknet_api::block::{BlockNumber, BlockTimestamp};
use std::sync::Arc;

/// Extension trait that provides execution capabilities on the madara backend.
pub trait MadaraBackendExecutionExt {
    /// Executor used for producing blocks.
    fn new_executor_for_block_production(
        self: &Arc<Self>,
        state_adaptor: LayeredStateAdaptor,
        block_info: BlockInfo,
    ) -> Result<TransactionExecutor<LayeredStateAdaptor>, Error>;
    /// Executor used for validating transactions on top of the pending block.
    fn new_transaction_validator(self: &Arc<Self>) -> Result<StatefulValidator<BlockifierStateAdapter>, Error>;
}

impl MadaraBackendExecutionExt for MadaraBackend {
    fn new_executor_for_block_production(
        self: &Arc<Self>,
        state_adaptor: LayeredStateAdaptor,
        block_info: BlockInfo,
    ) -> Result<TransactionExecutor<LayeredStateAdaptor>, Error> {
        Ok(TransactionExecutor::new(
            state_adaptor.into(),
            BlockContext::new(
                block_info,
                self.chain_config().blockifier_chain_info(),
                self.chain_config().exec_constants_by_protocol_version(self.chain_config().latest_protocol_version)?,
                self.chain_config().bouncer_config.clone(),
            ),
            TransactionExecutorConfig {
                concurrency_config: self.chain_config().block_production_concurrency.blockifier_config(),
            },
        ))
    }

    fn new_transaction_validator(self: &Arc<Self>) -> Result<StatefulValidator<BlockifierStateAdapter>, Error> {
        let pending_block = self.latest_pending_block();
        let block_n = self.get_latest_block_n()?.map(|n| n + 1).unwrap_or(/* genesis */ 0);
        Ok(StatefulValidator::create(
            CachedState::new(BlockifierStateAdapter::new(Arc::clone(self), block_n, Some(DbBlockId::Pending))),
            BlockContext::new(
                BlockInfo {
                    block_number: BlockNumber(block_n),
                    block_timestamp: BlockTimestamp(pending_block.header.block_timestamp.0),
                    sequencer_address: pending_block
                        .header
                        .sequencer_address
                        .try_into()
                        .map_err(|_| Error::InvalidSequencerAddress(pending_block.header.sequencer_address))?,
                    gas_prices: (&pending_block.header.l1_gas_price).into(),
                    use_kzg_da: pending_block.header.l1_da_mode == L1DataAvailabilityMode::Blob,
                },
                self.chain_config().blockifier_chain_info(),
                self.chain_config().exec_constants_by_protocol_version(pending_block.header.protocol_version)?,
                self.chain_config().bouncer_config.clone(),
            ),
        ))
    }
}

// TODO: deprecate this struct (only used for reexecution, which IMO should also go into MadaraBackendExecutionExt)
pub struct ExecutionContext {
    pub(crate) backend: Arc<MadaraBackend>,
    pub(crate) block_context: BlockContext,
    /// None means we are executing the genesis block. (no latest block)
    pub(crate) latest_visible_block: Option<DbBlockId>,
}

impl ExecutionContext {
    pub fn executor_for_block_production(&self) -> TransactionExecutor<BlockifierStateAdapter> {
        TransactionExecutor::new(
            self.init_cached_state(),
            self.block_context.clone(),
            TransactionExecutorConfig { concurrency_config: Default::default() },
        )
    }

    pub fn tx_validator(&self) -> StatefulValidator<BlockifierStateAdapter> {
        StatefulValidator::create(self.init_cached_state(), self.block_context.clone())
    }

    pub fn init_cached_state(&self) -> CachedState<BlockifierStateAdapter> {
        tracing::debug!(
            "Init cached state on top of {:?}, block number {:?}",
            self.latest_visible_block,
            self.block_context.block_info().block_number.0
        );

        CachedState::new(BlockifierStateAdapter::new(
            Arc::clone(&self.backend),
            self.block_context.block_info().block_number.0,
            self.latest_visible_block,
        ))
    }

    /// Init execution at the beginning of a block. The header of the block will be used, but all of the
    /// transactions' state modifications will not be visible.
    ///
    /// This function is usually what you would want for the `trace` rpc enpoints, for example.
    #[tracing::instrument(skip(backend, block_info), fields(module = "ExecutionContext"))]
    pub fn new_at_block_start(
        backend: Arc<MadaraBackend>,
        block_info: &MadaraMaybePendingBlockInfo,
    ) -> Result<Self, Error> {
        let (latest_visible_block, header_block_id) = match block_info {
            MadaraMaybePendingBlockInfo::Pending(_block) => {
                let latest_block_n = backend.get_latest_block_n()?;
                (
                    latest_block_n.map(DbBlockId::Number),
                    // when the block is pending, we use the latest block n + 1 to make the block header
                    // if there is no latest block, the pending block is actually the genesis block
                    latest_block_n.map(|el| el + 1).unwrap_or(0),
                )
            }
            MadaraMaybePendingBlockInfo::NotPending(block) => {
                // If the block is genesis, latest visible block is None.
                (block.header.block_number.checked_sub(1).map(DbBlockId::Number), block.header.block_number)
            }
        };
        Self::new(backend, block_info, latest_visible_block, header_block_id)
    }

    /// Init execution on top of a block. All of the transactions' state modifications are visible
    /// but the execution still happens within that block.
    /// This is essentially as if we're executing on top of the block after all of the transactions
    /// are executed, but before we switched to making a new block.
    ///
    /// This function is usually what you would want for the `estimateFee`, `simulateTransaction`, `call` rpc endpoints, for example.
    #[tracing::instrument(skip(backend, block_info), fields(module = "ExecutionContext"))]
    pub fn new_at_block_end(
        backend: Arc<MadaraBackend>,
        block_info: &MadaraMaybePendingBlockInfo,
    ) -> Result<Self, Error> {
        let (latest_visible_block, header_block_id) = match block_info {
            MadaraMaybePendingBlockInfo::Pending(_block) => {
                let latest_block_n = backend.get_latest_block_n()?;
                (Some(DbBlockId::Pending), latest_block_n.map(|el| el + 1).unwrap_or(0))
            }
            MadaraMaybePendingBlockInfo::NotPending(block) => {
                (Some(DbBlockId::Number(block.header.block_number)), block.header.block_number)
            }
        };
        Self::new(backend, block_info, latest_visible_block, header_block_id)
    }

    fn new(
        backend: Arc<MadaraBackend>,
        block_info: &MadaraMaybePendingBlockInfo,
        latest_visible_block: Option<DbBlockId>,
        block_number: u64,
    ) -> Result<Self, Error> {
        let (protocol_version, block_timestamp, sequencer_address, l1_gas_price, l1_da_mode) = match block_info {
            MadaraMaybePendingBlockInfo::Pending(block) => (
                block.header.protocol_version,
                block.header.block_timestamp,
                block.header.sequencer_address,
                block.header.l1_gas_price.clone(),
                block.header.l1_da_mode,
            ),
            MadaraMaybePendingBlockInfo::NotPending(block) => (
                block.header.protocol_version,
                block.header.block_timestamp,
                block.header.sequencer_address,
                block.header.l1_gas_price.clone(),
                block.header.l1_da_mode,
            ),
        };

        let versioned_constants = backend.chain_config().exec_constants_by_protocol_version(protocol_version)?;
        let chain_info = backend.chain_config().blockifier_chain_info();
        let block_info = BlockInfo {
            block_number: BlockNumber(block_number),
            block_timestamp: BlockTimestamp(block_timestamp.0),
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
            latest_visible_block,
            backend,
        })
    }
}
