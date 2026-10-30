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
use mp_chain_config::{L1DataAvailabilityMode, StarknetVersion};
use starknet_api::{
    block::{BlockInfo, BlockNumber, BlockTimestamp},
    core::ContractAddress,
};
use std::sync::Arc;

/// Builds historical execution context with the node's read permission and authenticated dataset provider.
fn block_context<D: MadaraStorageRead>(
    backend: &Arc<MadaraBackend<D>>,
    block_info: MadaraMaybePreconfirmedBlockInfo,
) -> Result<Arc<BlockContext>, Error> {
    let chain_config = backend.chain_config();
    let protocol_version = block_info.protocol_version();
    let mut context = BlockContext::new(
        BlockInfo {
            block_number: BlockNumber(block_info.block_number()),
            block_timestamp: BlockTimestamp(block_info.block_timestamp().0),
            starknet_version: protocol_version
                .to_blockifier()
                .map_err(|error| Error::UnsupportedStarknetVersion { version: *protocol_version, error })?,
            sequencer_address: ContractAddress::try_from(*block_info.sequencer_address())
                .map_err(|_| Error::InvalidSequencerAddress(*block_info.sequencer_address()))?,
            gas_prices: block_info.gas_prices().into(),
            use_kzg_da: *block_info.l1_da_mode() == L1DataAvailabilityMode::Blob,
        },
        chain_config.blockifier_chain_info(),
        chain_config.exec_constants_by_protocol_version(*protocol_version)?,
        chain_config.bouncer_config.clone(),
    );
    context.use_committed_data = backend.use_committed_data();
    context.committed_data_witnesses =
        Arc::new(blockifier::execution::syscalls::committed_data::CommittedDataWitnesses::from_provider(
            backend.committed_data_provider_at(context.block_info().block_number.0),
        ));
    Ok(context.into())
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

    pub fn get_initial_reads(
        &self,
    ) -> Result<blockifier::state::cached_state::StateMaps, blockifier::state::errors::StateError> {
        self.state.get_initial_reads()
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
            block_context: block_context(self.backend(), block_info)?,
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
            block_context: block_context(self.backend(), block_info)?, // ..but use the current block context
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
        let mut context = BlockContext::new(
            block_info,
            self.chain_config().blockifier_chain_info(),
            self.chain_config().exec_constants_by_protocol_version(self.chain_config().latest_protocol_version)?,
            self.chain_config().bouncer_config.clone(),
        );
        context.use_committed_data = self.use_committed_data();
        context.committed_data_witnesses =
            Arc::new(blockifier::execution::syscalls::committed_data::CommittedDataWitnesses::from_provider(
                self.committed_data_provider_at(context.block_info().block_number.0),
            ));
        Ok(TransactionExecutor::new(
            CachedState::new(state_adaptor),
            context,
            TransactionExecutorConfig {
                concurrency_config: self.chain_config().block_production_concurrency.blockifier_config(),
                stack_size: DEFAULT_STACK_SIZE,
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockifier::execution::syscalls::committed_data::{read_value, CommittedDataSet};
    use mc_db::{committed_data_retention::CommittedDataRetentionConfig, MadaraBackendConfig, MadaraStorageWrite};
    use mp_convert::Felt;

    /// Exercise the exact block-start context used by traceTransaction/traceBlockTransactions,
    /// including the real syscall read path after the durable proof index is pruned.
    /// This covers trace context/provider integration, not HTTP trace serialization.
    #[rstest::rstest]
    #[case::sequencer(false)]
    #[case::replica(true)]
    #[tokio::test]
    async fn committed_data_historical_trace_context_reads_pruned_values(#[case] replica: bool) {
        let backend = MadaraBackend::open_for_testing_with_config(
            mp_chain_config::ChainConfig::madara_test().into(),
            MadaraBackendConfig {
                use_committed_data: true,
                committed_data_retention: CommittedDataRetentionConfig {
                    proof_retention_secs: Some(0),
                    cache_bytes: 0,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let entries = vec![(Felt::ONE, Felt::MAX), (Felt::TWO, Felt::ZERO)];
        let tree = CommittedDataSet::new(entries.clone()).unwrap();
        // A replica imports the complete map through the same authenticated snapshot path.
        if replica {
            backend.import_committed_data_snapshot(tree.root(), entries).await.unwrap();
        } else {
            backend.db.write_committed_data_dataset(&tree, None).unwrap();
        }
        mc_db::test_utils::add_test_block(&backend, 0, vec![]);
        mc_db::test_utils::add_test_block(&backend, 1, vec![]);
        backend.set_latest_l1_confirmed(Some(1)).unwrap();
        let view = backend.view_on_confirmed(0).unwrap();
        let context = view.block_view_on_latest().unwrap().new_execution_context_at_block_start().unwrap();
        let read = || {
            let mut gas = u64::MAX;
            let mut failure = None;
            let result = read_value(
                context.block_context.use_committed_data,
                &context.block_context.committed_data_witnesses,
                starknet_api::abi::abi_utils::selector_from_name("get_value").0,
                &[tree.root(), Felt::ONE],
                &mut gas,
                &mut failure,
            )
            .unwrap();
            assert!(failure.is_none());
            result
        };
        let before = read();
        assert!(backend.db.maintain_committed_data_history(Some(u64::MAX)).unwrap());
        assert_eq!(read(), before);
        assert_eq!(before.0, Felt::MAX);
        // Historical execution below the cutoff must not reactivate the proof index.
        assert!(!backend.db.maintain_committed_data_history(Some(u64::MAX)).unwrap());
    }
}
