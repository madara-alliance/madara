use anyhow::Context;
use blockifier::{
    blockifier::transaction_executor::TransactionExecutor, state::state_api::State,
    transaction::transaction_execution::Transaction,
};
use mc_db::MadaraBackend;
use mc_exec::{LayeredStateAdapter, MadaraBackendExecutionExt};
use mp_block::header::{BlockTimestamp, GasPrices, PreconfirmedHeader};
use mp_chain_config::{L1DataAvailabilityMode, StarknetVersion};
use mp_class::ConvertedClass;
use mp_convert::{Felt, ToFelt};
use mp_transactions::validated::TxTimestamp;
use starknet_api::StarknetApiError;
use std::{
    collections::VecDeque,
    ops::{Add, AddAssign},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

// TODO: add these to metrics
#[derive(Default, Clone, Debug)]
pub struct ExecutionStats {
    /// Number of batches executed before reaching the bouncer capacity.
    pub n_batches: usize,
    /// Number of transactions included into the block.
    pub n_added_to_block: usize,
    /// Number of transactions executed.
    pub n_executed: usize,
    /// Reverted transactions are failing transactions that are included in the block.
    pub n_reverted: usize,
    /// Rejected are txs are failing transactions that are not revertible. They are thus not included in the block
    pub n_rejected: usize,
    /// Number of declared classes.
    pub declared_classes: usize,
    /// Total L2 gas consumed by the transactions in the block.
    pub l2_gas_consumed: u128,
    /// Execution time
    pub exec_duration: Duration,
}

impl Add for ExecutionStats {
    type Output = Self;
    fn add(self, other: Self) -> Self::Output {
        Self {
            n_batches: self.n_batches + other.n_batches,
            n_added_to_block: self.n_added_to_block + other.n_added_to_block,
            n_executed: self.n_executed + other.n_executed,
            n_reverted: self.n_reverted + other.n_reverted,
            n_rejected: self.n_rejected + other.n_rejected,
            declared_classes: self.declared_classes + other.declared_classes,
            l2_gas_consumed: self.l2_gas_consumed + other.l2_gas_consumed,
            exec_duration: self.exec_duration + other.exec_duration,
        }
    }
}
impl AddAssign for ExecutionStats {
    fn add_assign(&mut self, rhs: Self) {
        *self = self.clone() + rhs
    }
}

#[derive(Default, Debug)]
pub(crate) struct BatchToExecute {
    pub txs: Vec<Transaction>,
    pub additional_info: VecDeque<AdditionalTxInfo>,
}

impl BatchToExecute {
    pub fn with_capacity(cap: usize) -> Self {
        Self { txs: Vec::with_capacity(cap), additional_info: VecDeque::with_capacity(cap) }
    }

    pub fn len(&self) -> usize {
        self.txs.len()
    }
    pub fn is_empty(&self) -> bool {
        self.txs.is_empty()
    }

    pub fn push(&mut self, tx: Transaction, additional_info: AdditionalTxInfo) {
        self.txs.push(tx);
        self.additional_info.push_back(additional_info);
    }

    pub fn remove_n_front(&mut self, n_to_remove: usize) -> BatchToExecute {
        // we can't actually use split_off because it doesnt leave the cap :/

        let txs = self.txs.drain(..n_to_remove).collect();
        let additional_info = self.additional_info.drain(..n_to_remove).collect();
        BatchToExecute { txs, additional_info }
    }
}

impl Extend<(Transaction, AdditionalTxInfo)> for BatchToExecute {
    fn extend<T: IntoIterator<Item = (Transaction, AdditionalTxInfo)>>(&mut self, iter: T) {
        for (tx, additional_info) in iter {
            self.push(tx, additional_info)
        }
    }
}

impl IntoIterator for BatchToExecute {
    type Item = (Transaction, AdditionalTxInfo);
    type IntoIter =
        std::iter::Zip<std::vec::IntoIter<Transaction>, std::collections::vec_deque::IntoIter<AdditionalTxInfo>>;
    fn into_iter(self) -> Self::IntoIter {
        self.txs.into_iter().zip(self.additional_info)
    }
}

impl FromIterator<(Transaction, AdditionalTxInfo)> for BatchToExecute {
    fn from_iter<T: IntoIterator<Item = (Transaction, AdditionalTxInfo)>>(iter: T) -> Self {
        let mut s = Self::default();
        s.extend(iter);
        s
    }
}

#[derive(Debug, Default)]
pub(crate) struct AdditionalTxInfo {
    pub declared_class: Option<ConvertedClass>,
    /// Earliest known timestamp for this transaction. Used for mempool re-insertion.
    pub arrived_at: TxTimestamp,
}

/// This is a pending header, without parent_block_hash. Parent block hash is not visible to the execution,
/// and in addition, we can't know it yet without closing the block and updating the global trie to compute
/// the global state root.
/// See [`crate::executor::Executor`]; we want to be able to start the execution of new blocks without waiting
/// on the earlier to be closed.
#[derive(Debug, Clone)]
pub(crate) struct BlockExecutionContext {
    /// The new block_n.
    pub block_number: u64,
    /// The Starknet address of the sequencer who created this block.
    pub sequencer_address: Felt,
    /// Unix timestamp (seconds) when the block was produced -- before executing any transaction.
    pub block_timestamp: SystemTime, // We use a systemtime here for better logging.
    /// The version of the Starknet protocol used when creating this block
    pub protocol_version: StarknetVersion,
    /// Gas prices for this block
    pub gas_prices: GasPrices,
    /// The mode of data availability for this block
    pub l1_da_mode: L1DataAvailabilityMode,
}

impl BlockExecutionContext {
    pub fn into_header(self) -> PreconfirmedHeader {
        PreconfirmedHeader {
            block_number: self.block_number,
            sequencer_address: self.sequencer_address,
            block_timestamp: self.block_timestamp.into(),
            protocol_version: self.protocol_version,
            gas_prices: self.gas_prices,
            l1_da_mode: self.l1_da_mode,
        }
    }

    pub fn to_blockifier(&self) -> Result<starknet_api::block::BlockInfo, StarknetApiError> {
        Ok(starknet_api::block::BlockInfo {
            block_number: starknet_api::block::BlockNumber(self.block_number),
            block_timestamp: starknet_api::block::BlockTimestamp(BlockTimestamp::from(self.block_timestamp).0),
            sequencer_address: self.sequencer_address.try_into()?,
            gas_prices: (&self.gas_prices).into(),
            use_kzg_da: self.l1_da_mode == L1DataAvailabilityMode::Blob,
        })
    }
}

pub(crate) fn create_execution_context(
    backend: &Arc<MadaraBackend>,
    block_n: u64,
    previous_l2_gas_price: u128,
    previous_l2_gas_used: u128,
) -> anyhow::Result<BlockExecutionContext> {
    let (block_timestamp, gas_prices) = if let Some(custom_header) =
        backend.get_custom_header().filter(|h| h.block_n == block_n)
    {
        // Convert Unix timestamp (seconds since Jan 1, 1970) to SystemTime
        let block_timestamp = UNIX_EPOCH + Duration::from_secs(custom_header.timestamp);
        let gas_prices = custom_header.gas_prices;
        (block_timestamp, gas_prices)
    } else {
        let l1_gas_quote = backend
            .get_last_l1_gas_quote()
            .context("No L1 gas quote available. Ensure that the L1 gas quote is set before calculating gas prices.")?;

        let gas_prices = backend.calculate_gas_prices(&l1_gas_quote, previous_l2_gas_price, previous_l2_gas_used)?;
        (SystemTime::now(), gas_prices)
    };

    Ok(BlockExecutionContext {
        sequencer_address: **backend.chain_config().sequencer_address,
        block_timestamp,
        protocol_version: backend.chain_config().latest_protocol_version,
        gas_prices,
        l1_da_mode: backend.chain_config().l1_da_mode,
        block_number: block_n,
    })
}

/// Creates a TransactionExecutor with the given execution context and state adapter,
/// and sets up the block_n-10 state diff entry if available.
///
/// This is a helper function to avoid code duplication between normal block production
/// and re-execution scenarios.
pub(crate) fn create_executor_with_block_n_min_10(
    backend: &Arc<MadaraBackend>,
    exec_ctx: &BlockExecutionContext,
    state_adaptor: LayeredStateAdapter,
    get_block_n_min_10_hash: impl FnOnce(u64) -> anyhow::Result<Option<(u64, Felt)>>,
) -> anyhow::Result<TransactionExecutor<LayeredStateAdapter>> {
    let mut executor = backend
        .new_executor_for_block_production(state_adaptor, exec_ctx.to_blockifier()?)
        .context("Creating TransactionExecutor")?;

    // Prepare the block_n-10 state diff entry on the 0x1 contract
    if let Some((block_n_min_10, block_hash_n_min_10)) = get_block_n_min_10_hash(exec_ctx.block_number)? {
        let contract_address = 1u64.into();
        let key = block_n_min_10.into();
        executor
            .block_state
            .as_mut()
            .expect("Blockifier block context has been taken")
            .set_storage_at(contract_address, key, block_hash_n_min_10)
            .context("Cannot set storage value in cache")?;

        tracing::debug!(
            "State diff inserted {:#x} {:#x} => {block_hash_n_min_10:#x}",
            contract_address.to_felt(),
            key.to_felt()
        );
    }

    Ok(executor)
}
