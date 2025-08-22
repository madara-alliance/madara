use crate::{blockifier_state_adapter::BlockifierStateAdapter, Error, LayeredStateAdapter};
use blockifier::{
    blockifier::{
        config::TransactionExecutorConfig,
        stateful_validator::StatefulValidator,
        transaction_executor::{TransactionExecutor, DEFAULT_STACK_SIZE},
    },
    context::BlockContext,
    state::cached_state::CachedState,
};
use mc_db::{MadaraBackend, MadaraBlockView, MadaraStateView, MadaraStorageRead};
use mp_block::MadaraMaybePreconfirmedBlockInfo;
use mp_chain_config::{ChainConfig, L1DataAvailabilityMode, StarknetVersion};
use starknet_api::{
    block::{BlockInfo, BlockNumber, BlockTimestamp},
    core::ContractAddress,
};
use std::sync::Arc;

fn block_context(
    chain_config: &ChainConfig,
    block_info: MadaraMaybePreconfirmedBlockInfo,
) -> Result<Arc<BlockContext>, Error> {
    Ok(BlockContext::new(
        BlockInfo {
            block_number: BlockNumber(block_info.block_number()),
            block_timestamp: BlockTimestamp(block_info.block_timestamp().0),
            sequencer_address: ContractAddress::try_from(*block_info.sequencer_address())
                .map_err(|_| Error::InvalidSequencerAddress(*block_info.sequencer_address()))?,
            gas_prices: block_info.l1_gas_price().into(),
            use_kzg_da: *block_info.l1_da_mode() == L1DataAvailabilityMode::Blob,
        },
        chain_config.blockifier_chain_info(),
        chain_config.exec_constants_by_protocol_version(*block_info.protocol_version())?,
        chain_config.bouncer_config.clone(),
    )
    .into())
}

pub struct ExecutionContext<D: MadaraStorageRead> {
    pub state: CachedState<BlockifierStateAdapter<D>>,
    pub block_context: Arc<BlockContext>,
    pub protocol_version: StarknetVersion,
}

impl<D: MadaraStorageRead> ExecutionContext<D> {
    pub fn view(&self) -> &MadaraStateView<D> {
        &self.state.state.view
    }

    pub fn into_transaction_validator(self) -> StatefulValidator<BlockifierStateAdapter<D>> {
        StatefulValidator::create(self.state, Arc::unwrap_or_clone(self.block_context))
    }
}

/// Extension trait that provides execution capabilities on the madara backend.
pub trait MadaraBlockViewExecutionExt<D: MadaraStorageRead> {
    fn new_execution_context(&self) -> Result<ExecutionContext<D>, Error>;

    /// Init execution at the beginning of a block. The header of the block will be used, but all of the
    /// transactions' state modifications will not be visible.
    ///
    /// This function is usually what you would want for the `trace` rpc enpoints, for example.
    fn new_execution_context_at_block_start(&self) -> Result<ExecutionContext<D>, Error>;
}

impl<D: MadaraStorageRead> MadaraBlockViewExecutionExt<D> for MadaraBlockView<D> {
    fn new_execution_context(&self) -> Result<ExecutionContext<D>, Error> {
        let block_info = self.get_block_info()?;
        Ok(ExecutionContext {
            protocol_version: *block_info.protocol_version(),
            state: CachedState::new(BlockifierStateAdapter::new(self.clone().into(), block_info.block_number())),
            block_context: block_context(self.backend().chain_config(), block_info)?,
        })
    }
    fn new_execution_context_at_block_start(&self) -> Result<ExecutionContext<D>, Error> {
        let block_info = self.get_block_info()?;
        Ok(ExecutionContext {
            protocol_version: *block_info.protocol_version(),
            state: CachedState::new(BlockifierStateAdapter::new(
                self.clone().state_view_on_parent(), // Only make the parent block state visible..
                block_info.block_number(),
            )),
            block_context: block_context(self.backend().chain_config(), block_info)?, // ..but use the current block context
        })
    }
}

/// Extension trait that provides execution capabilities on the madara backend.
pub trait MadaraBackendExecutionExt<D: MadaraStorageRead> {
    /// Executor used for producing blocks.
    fn new_executor_for_block_production(
        self: &Arc<Self>,
        state_adaptor: LayeredStateAdapter<D>,
        block_info: BlockInfo,
    ) -> Result<TransactionExecutor<LayeredStateAdapter<D>>, Error>;
}

impl<D: MadaraStorageRead> MadaraBackendExecutionExt<D> for MadaraBackend<D> {
    fn new_executor_for_block_production(
        self: &Arc<Self>,
        state_adaptor: LayeredStateAdapter<D>,
        block_info: BlockInfo,
    ) -> Result<TransactionExecutor<LayeredStateAdapter<D>>, Error> {
        Ok(TransactionExecutor::new(
            CachedState::new(state_adaptor),
            BlockContext::new(
                block_info,
                self.chain_config().blockifier_chain_info(),
                self.chain_config().exec_constants_by_protocol_version(self.chain_config().latest_protocol_version)?,
                self.chain_config().bouncer_config.clone(),
            ),
            TransactionExecutorConfig {
                concurrency_config: self.chain_config().block_production_concurrency.blockifier_config(),
                stack_size: DEFAULT_STACK_SIZE,
            },
        ))
    }
}
