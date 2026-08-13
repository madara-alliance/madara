use crate::fallback::types::ExecutionMode;
use anyhow::Context;
use blockifier::{
    blockifier::transaction_executor::TransactionExecutor, state::state_api::State,
    transaction::transaction_execution::Transaction,
};
use mc_db::MadaraBackend;
use mc_exec::LayeredStateAdapter;
use mp_block::header::{BlockTimestamp, GasPrices, PreconfirmedHeader};
use mp_chain_config::{L1DataAvailabilityMode, StarknetVersion};
use mp_class::ConvertedClass;
use mp_convert::{Felt, ToFelt};
use mp_transactions::validated::TxTimestamp;
use starknet_api::StarknetApiError;
use std::collections::HashSet;
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
    /// Number of included transactions executed by Blockifier.
    pub n_added_by_blockifier: usize,
    /// Number of included transactions executed by Rust Exec.
    pub n_added_by_rust_exec: usize,
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
    /// Blockifier execution time.
    pub blockifier_exec_duration: Duration,
    /// Rust Exec execution time.
    pub rust_exec_duration: Duration,
}

impl Add for ExecutionStats {
    type Output = Self;
    fn add(self, other: Self) -> Self::Output {
        Self {
            n_batches: self.n_batches + other.n_batches,
            n_added_to_block: self.n_added_to_block + other.n_added_to_block,
            n_added_by_blockifier: self.n_added_by_blockifier + other.n_added_by_blockifier,
            n_added_by_rust_exec: self.n_added_by_rust_exec + other.n_added_by_rust_exec,
            n_executed: self.n_executed + other.n_executed,
            n_reverted: self.n_reverted + other.n_reverted,
            n_rejected: self.n_rejected + other.n_rejected,
            declared_classes: self.declared_classes + other.declared_classes,
            l2_gas_consumed: self.l2_gas_consumed + other.l2_gas_consumed,
            exec_duration: self.exec_duration + other.exec_duration,
            blockifier_exec_duration: self.blockifier_exec_duration + other.blockifier_exec_duration,
            rust_exec_duration: self.rust_exec_duration + other.rust_exec_duration,
        }
    }
}
impl AddAssign for ExecutionStats {
    fn add_assign(&mut self, rhs: Self) {
        *self = self.clone() + rhs
    }
}

#[derive(Default, Debug, Clone)]
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

#[derive(Debug, Clone)]
pub(crate) struct TaintedRebuildCarryTx {
    pub tx: Transaction,
    pub additional_info: AdditionalTxInfo,
    pub source_block_n: Option<u64>,
}

#[derive(Debug, Default, Clone)]
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
            starknet_version: self.protocol_version.to_blockifier()?,
            use_kzg_da: self.l1_da_mode == L1DataAvailabilityMode::Blob,
        })
    }

    /// Create a BlockContext from this execution context with the given chain config and exec constants.
    /// This is a helper function that encapsulates the BlockContext creation logic.
    pub fn to_block_context(
        &self,
        chain_config: &Arc<mp_chain_config::ChainConfig>,
        exec_constants: &blockifier::blockifier_versioned_constants::VersionedConstants,
    ) -> anyhow::Result<blockifier::context::BlockContext> {
        use blockifier::context::BlockContext;
        let block_info = self.to_blockifier()?;
        Ok(BlockContext::new(
            block_info,
            chain_config.blockifier_chain_info(),
            exec_constants.clone(),
            chain_config.bouncer_config.clone(),
        ))
    }
}

pub(crate) fn create_execution_context(
    backend: &Arc<MadaraBackend>,
    block_n: u64,
    previous_l2_gas_price: u128,
    previous_l2_gas_used: u128,
) -> anyhow::Result<BlockExecutionContext> {
    let (block_timestamp, gas_prices) = if let Some(custom_header) = backend.get_custom_header(block_n) {
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
/// and re-execution scenarios. It reuses `backend.new_executor_for_block_production()`
/// but allows using a custom chain_config (e.g., saved config for re-execution).
pub(crate) fn create_executor_with_block_n_min_10(
    backend: &Arc<MadaraBackend>,
    exec_ctx: &BlockExecutionContext,
    state_adaptor: LayeredStateAdapter,
    get_block_n_min_10_hash: impl FnOnce(u64) -> anyhow::Result<Option<(u64, Felt)>>,
    custom_chain_config: Option<&Arc<mp_chain_config::ChainConfig>>,
) -> anyhow::Result<TransactionExecutor<LayeredStateAdapter>> {
    use mc_exec::MadaraBackendExecutionExt;

    // Use backend.new_executor_for_block_production() to create executor (reuses existing logic)
    let block_info = exec_ctx.to_blockifier()?;
    let mut executor = backend
        .clone()
        .new_executor_for_block_production(state_adaptor, block_info.clone())
        .context("Creating TransactionExecutor")?;

    // If custom_chain_config is provided, override BlockContext to use it instead of backend's config
    // This is needed for re-execution scenarios where we want to use saved config
    if let Some(chain_config) = custom_chain_config {
        // Get exec_constants from custom chain_config
        let exec_constants = chain_config
            .exec_constants_by_protocol_version(exec_ctx.protocol_version)
            .context("Failed to resolve execution constants for protocol version")?;

        // Use to_block_context helper to create BlockContext (reuses existing logic)
        let block_context = exec_ctx.to_block_context(chain_config, &exec_constants)?;
        executor.block_context = Arc::new(block_context);
        // Override concurrency config as well
        executor.config.concurrency_config = chain_config.block_production_concurrency.blockifier_config();
    }

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

// ─────────────────────────────────────────────────────────────────────────────
// Step 2: RustExec routing types
// ─────────────────────────────────────────────────────────────────────────────

/// Configuration for routing transactions to RustExec vs Blockifier.
/// Kept separate from ChainConfig per design doc 02 §Config placement.
#[derive(Debug, Clone)]
pub(crate) struct RustExecRoutingConfig {
    /// Whitelisted sender addresses for RustExec routing.
    pub executor_addresses: HashSet<Felt>,
    /// Whitelisted call selectors for RustExec routing.
    pub supported_selectors: HashSet<Felt>,
    /// Whitelisted contract class hashes for RustExec routing.
    pub supported_class_hashes: HashSet<Felt>,
    /// Max RustExec txs per batcher cycle (default placeholder: 30).
    pub rust_batch_size: usize,
    /// Max Blockifier txs per batcher cycle (default placeholder: 10).
    pub blockifier_batch_size: usize,
    pub runtime_options: RustExecRuntimeOptions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RustExecRuntimeOptions {
    pub conversion_log: bool,
    pub execution_log: bool,
    pub execution_log_inner: bool,
    pub tx_diff_log: bool,
    pub debug_block: Option<u64>,
    pub inner_timing_log: bool,
    pub ctx_cache: bool,
    pub pedersen_cache: bool,
    pub precomputed_sn_keccak: bool,
    pub hash_agg_logs: bool,
    pub storage_agg_logs: bool,
    pub ignore_fee_mismatch: bool,
    pub ignore_fee_token_mismatch: bool,
    pub ignored_storage_mismatch_canonical_source: RustExecCanonicalSource,
    pub settle_trade_v3_positions: Option<u16>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RustExecCanonicalSource {
    #[default]
    ExecutionBox,
    BlockifierReexec,
}

impl Default for RustExecRuntimeOptions {
    fn default() -> Self {
        Self {
            conversion_log: false,
            execution_log: false,
            execution_log_inner: false,
            tx_diff_log: false,
            debug_block: None,
            inner_timing_log: false,
            ctx_cache: true,
            pedersen_cache: true,
            precomputed_sn_keccak: false,
            hash_agg_logs: false,
            storage_agg_logs: false,
            ignore_fee_mismatch: false,
            ignore_fee_token_mismatch: true,
            ignored_storage_mismatch_canonical_source: RustExecCanonicalSource::ExecutionBox,
            settle_trade_v3_positions: None,
        }
    }
}

impl Default for RustExecRoutingConfig {
    fn default() -> Self {
        Self {
            executor_addresses: HashSet::new(),
            supported_selectors: mc_rust_exec::supported_selectors(),
            supported_class_hashes: mc_rust_exec::supported_class_hashes(),
            rust_batch_size: 1,
            blockifier_batch_size: 1,
            runtime_options: RustExecRuntimeOptions::default(),
        }
    }
}

pub(crate) fn build_rust_exec_runtime_config(
    routing_cfg: &RustExecRoutingConfig,
    no_charge_fee: bool,
) -> mc_rust_exec::RustExecRuntimeConfig {
    mc_rust_exec::RustExecRuntimeConfig {
        supported_contract_class_hashes: routing_cfg.supported_class_hashes.clone(),
        no_charge_fee,
        conversion_log: routing_cfg.runtime_options.conversion_log,
        execution_log: routing_cfg.runtime_options.execution_log,
        execution_log_inner: routing_cfg.runtime_options.execution_log_inner,
        tx_diff_log: routing_cfg.runtime_options.tx_diff_log,
        debug_block: routing_cfg.runtime_options.debug_block,
        inner_timing_log: routing_cfg.runtime_options.inner_timing_log,
        ctx_cache: routing_cfg.runtime_options.ctx_cache,
        pedersen_cache: routing_cfg.runtime_options.pedersen_cache,
        precomputed_sn_keccak: routing_cfg.runtime_options.precomputed_sn_keccak,
        hash_agg_logs: routing_cfg.runtime_options.hash_agg_logs,
        storage_agg_logs: routing_cfg.runtime_options.storage_agg_logs,
        ignore_fee_mismatch: routing_cfg.runtime_options.ignore_fee_mismatch,
        settle_trade_v3_positions: routing_cfg.runtime_options.settle_trade_v3_positions,
        ..Default::default()
    }
}

/// Per-call routing decision returned by the classifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RouteDecision {
    Rust,
    Blockifier(RouteFallbackReason),
}

/// Reason a transaction was routed to Blockifier instead of RustExec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RouteFallbackReason {
    NotInvoke,
    SenderNotExecutor,
    SelectorNotSupported,
    ClassHashLookupFailed,
    ClassHashNotSupported,
    MulticallPartiallySupported,
}

/// Dual-batch payload emitted by batcher each cycle.
/// rust_batch is empty by construction when mode == BlockifierOnly.
#[derive(Debug)]
pub(crate) struct RoutedBatchToExecute {
    pub rust_batch: BatchToExecute,
    pub blockifier_batch: BatchToExecute,
    /// Initial execution-frontier block number captured by the batcher when this routed
    /// payload was built. The executor rebinds this to the authoritative block frontier
    /// once the payload is actually accepted and whenever a deferred suffix rolls forward.
    pub block_n: u64,
    /// Execution mode captured by the batcher when this routed payload was built.
    /// The executor uses this to normalize queued payloads across idle-boundary
    /// mode changes before freezing the next block snapshot.
    pub execution_mode: ExecutionMode,
    /// Execution epoch captured by the batcher when this routed payload was built.
    /// Used to recover stale in-flight routed batches after fallback.
    pub execution_epoch: u64,
}

impl Default for RoutedBatchToExecute {
    fn default() -> Self {
        Self {
            rust_batch: BatchToExecute::default(),
            blockifier_batch: BatchToExecute::default(),
            block_n: 0,
            execution_mode: ExecutionMode::BlockifierOnly,
            execution_epoch: 0,
        }
    }
}

impl RoutedBatchToExecute {
    pub fn is_empty(&self) -> bool {
        self.rust_batch.is_empty() && self.blockifier_batch.is_empty()
    }
    pub fn total_len(&self) -> usize {
        self.rust_batch.len() + self.blockifier_batch.len()
    }
}

/// Convenience conversion: all items go to `blockifier_batch`.
/// Used by executor tests and other code that creates a single-branch payload.
impl FromIterator<(Transaction, AdditionalTxInfo)> for RoutedBatchToExecute {
    fn from_iter<T: IntoIterator<Item = (Transaction, AdditionalTxInfo)>>(iter: T) -> Self {
        let blockifier_batch = iter.into_iter().collect::<BatchToExecute>();
        Self {
            blockifier_batch,
            rust_batch: BatchToExecute::default(),
            block_n: 0,
            execution_mode: ExecutionMode::BlockifierOnly,
            execution_epoch: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::time::Duration;

    use starknet_types_core::felt::Felt;

    use super::{build_rust_exec_runtime_config, ExecutionStats, RustExecRoutingConfig};

    #[test]
    fn execution_stats_preserve_per_engine_accounting() {
        let blockifier = ExecutionStats {
            n_added_to_block: 2,
            n_added_by_blockifier: 2,
            blockifier_exec_duration: Duration::from_millis(20),
            ..Default::default()
        };
        let rust_exec = ExecutionStats {
            n_added_to_block: 3,
            n_added_by_rust_exec: 3,
            rust_exec_duration: Duration::from_millis(5),
            ..Default::default()
        };

        let combined = blockifier + rust_exec;
        assert_eq!(combined.n_added_to_block, 5);
        assert_eq!(combined.n_added_by_blockifier, 2);
        assert_eq!(combined.n_added_by_rust_exec, 3);
        assert_eq!(combined.blockifier_exec_duration, Duration::from_millis(20));
        assert_eq!(combined.rust_exec_duration, Duration::from_millis(5));
    }

    #[test]
    fn runtime_config_carries_supported_rust_class_hashes() {
        let class_hash_a = Felt::from_hex_unchecked("0x123");
        let class_hash_b = Felt::from_hex_unchecked("0x456");
        let routing_cfg = RustExecRoutingConfig {
            supported_class_hashes: HashSet::from([class_hash_a, class_hash_b]),
            ..Default::default()
        };

        let runtime_cfg = build_rust_exec_runtime_config(&routing_cfg, true);

        assert_eq!(runtime_cfg.supported_contract_class_hashes, HashSet::from([class_hash_a, class_hash_b]));
        assert!(runtime_cfg.no_charge_fee);
    }

    #[test]
    fn routing_config_derives_supported_contracts_from_manifest() {
        let routing_cfg = RustExecRoutingConfig::default();

        assert_eq!(routing_cfg.supported_class_hashes, mc_rust_exec::supported_class_hashes());
        assert_eq!(routing_cfg.supported_selectors, mc_rust_exec::supported_selectors());
    }
}
