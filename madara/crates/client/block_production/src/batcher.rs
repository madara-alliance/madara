use crate::classifier::classify_invoke;
use crate::fallback::types::ExecutionMode;
use crate::metrics::BlockProductionMetrics;
use crate::util::{
    AdditionalTxInfo, BatchToExecute, RouteDecision, RouteFallbackReason, RoutedBatchToExecute, RustExecRoutingConfig,
};
use crate::MempoolIntakeMode;
use anyhow::Context;
use futures::{
    stream::{self, BoxStream, PollNext},
    StreamExt, TryStreamExt,
};
use mc_db::MadaraBackend;
use mc_mempool::Mempool;
use mc_settlement_client::SettlementClient;
use mp_convert::ToFelt;
use mp_transactions::{
    validated::{TxTimestamp, ValidatedTransaction},
    L1HandlerTransactionWithFee, Transaction as MpTransaction,
};
use mp_utils::service::ServiceContext;
use opentelemetry::KeyValue;
use std::{collections::VecDeque, sync::Arc, time::Duration};
use tokio::sync::mpsc;
use tokio::sync::watch;
use tokio::time::Instant;

const BATCHER_OUTPUT_BACKPRESSURE_INFO_MS: f64 = 50.0;

#[derive(Default)]
struct ValidatedBatchBoundary {
    first_hash: Option<String>,
    first_nonce: Option<String>,
    last_hash: Option<String>,
    last_nonce: Option<String>,
}

fn validated_nonce_string(tx: &ValidatedTransaction) -> Option<String> {
    match &tx.transaction {
        MpTransaction::Invoke(tx) => Some(format!("{:#x}", tx.nonce())),
        MpTransaction::Declare(tx) => Some(format!("{:#x}", tx.nonce())),
        MpTransaction::DeployAccount(tx) => Some(format!("{:#x}", tx.nonce())),
        MpTransaction::L1Handler(tx) => Some(format!("{:#x}", tx.nonce)),
        MpTransaction::Deploy(_) => None,
    }
}

fn summarize_validated_batch(txs: &[ValidatedTransaction]) -> ValidatedBatchBoundary {
    let mut summary = ValidatedBatchBoundary::default();
    if let Some(first) = txs.first() {
        summary.first_hash = Some(format!("{:#x}", first.hash));
        summary.first_nonce = validated_nonce_string(first);
    }
    if let Some(last) = txs.last() {
        summary.last_hash = Some(format!("{:#x}", last.hash));
        summary.last_nonce = validated_nonce_string(last);
    }
    summary
}

struct RoutedPickOutcome {
    routed: RoutedBatchToExecute,
    routed_recoverable_txs: Vec<ValidatedTransaction>,
    deferred_txs: Vec<ValidatedTransaction>,
    cap_stop_branch: Option<&'static str>,
}

fn drain_local_carry(local_carry: &mut VecDeque<ValidatedTransaction>, max_items: usize) -> Vec<ValidatedTransaction> {
    let mut picked = Vec::with_capacity(max_items);
    for _ in 0..max_items {
        let Some(tx) = local_carry.pop_front() else { break };
        picked.push(tx);
    }
    picked
}

fn prepend_local_carry(
    local_carry: &mut VecDeque<ValidatedTransaction>,
    txs: impl IntoIterator<Item = ValidatedTransaction>,
) {
    let mut prefix = txs.into_iter().collect::<VecDeque<_>>();
    if prefix.is_empty() {
        return;
    }
    prefix.append(local_carry);
    *local_carry = prefix;
}

pub struct Batcher {
    backend: Arc<MadaraBackend>,
    mempool: Arc<Mempool>,
    l1_message_stream: BoxStream<'static, anyhow::Result<L1HandlerTransactionWithFee>>,
    ctx: ServiceContext,
    out: mpsc::Sender<RoutedBatchToExecute>,
    bypass_in: mpsc::Receiver<ValidatedTransaction>,
    mempool_intake_rx: watch::Receiver<MempoolIntakeMode>,
    mempool_intake_tx: watch::Sender<MempoolIntakeMode>,
    tainted_rebuild_active_rx: watch::Receiver<bool>,
    execution_mode_rx: watch::Receiver<ExecutionMode>,
    execution_epoch_rx: watch::Receiver<u64>,
    routing_cfg: RustExecRoutingConfig,
    metrics: Arc<BlockProductionMetrics>,
    batch_size: usize,
    replay_mode_enabled: bool,
}

fn l1_to_recoverable_validated(tx: L1HandlerTransactionWithFee, chain_id: mp_convert::Felt) -> ValidatedTransaction {
    let hash = tx.tx.compute_hash(chain_id, /* offset_version */ false, /* legacy */ false);
    ValidatedTransaction {
        transaction: MpTransaction::L1Handler(tx.tx.clone()),
        paid_fee_on_l1: Some(tx.paid_fee_on_l1),
        contract_address: tx.tx.contract_address,
        arrived_at: TxTimestamp::now(),
        declared_class: None,
        hash,
        charge_fee: false,
    }
}

impl Batcher {
    fn pick_limit_for_mode(&self, execution_mode: ExecutionMode) -> usize {
        match execution_mode {
            ExecutionMode::BlockifierOnly => self.batch_size.min(self.routing_cfg.blockifier_batch_size),
            ExecutionMode::Mixed => self
                .batch_size
                .min(self.routing_cfg.blockifier_batch_size.saturating_add(self.routing_cfg.rust_batch_size)),
        }
        .max(1)
    }

    fn route_picked_txs(
        &self,
        picked: Vec<ValidatedTransaction>,
        execution_mode: ExecutionMode,
        frontier_block_n: u64,
        execution_epoch: u64,
    ) -> anyhow::Result<RoutedPickOutcome> {
        let picked_total = picked.len();
        let mut routed_recoverable_txs = Vec::with_capacity(picked_total);
        let mut deferred_txs = Vec::new();
        let mut cap_stop_branch = None;
        let mut routed = RoutedBatchToExecute {
            rust_batch: BatchToExecute::with_capacity(self.routing_cfg.rust_batch_size),
            blockifier_batch: BatchToExecute::with_capacity(self.routing_cfg.blockifier_batch_size),
            block_n: frontier_block_n,
            execution_mode,
            execution_epoch,
        };

        let mut picked_iter = picked.into_iter();
        while let Some(validated) = picked_iter.next() {
            let recovery_tx = validated.clone();
            let (tx, arrived_at, declared_class) = validated
                .into_blockifier_for_sequencing()
                .map_err(anyhow::Error::from)
                .context("Converting picked transaction for executor handoff")?;
            let info = AdditionalTxInfo { declared_class, arrived_at };

            match execution_mode {
                ExecutionMode::BlockifierOnly => {
                    if routed.blockifier_batch.len() >= self.routing_cfg.blockifier_batch_size {
                        cap_stop_branch = Some("blockifier");
                        deferred_txs.push(recovery_tx);
                        deferred_txs.extend(picked_iter);
                        break;
                    }
                    routed_recoverable_txs.push(recovery_tx);
                    routed.blockifier_batch.push(tx, info);
                    self.metrics.batcher_routed_blockifier_total.add(1, &[]);
                }
                ExecutionMode::Mixed => match classify_invoke(&tx, &self.routing_cfg, &self.backend) {
                    RouteDecision::Rust => {
                        if routed.rust_batch.len() >= self.routing_cfg.rust_batch_size {
                            cap_stop_branch = Some("rust");
                            deferred_txs.push(recovery_tx);
                            deferred_txs.extend(picked_iter);
                            break;
                        }
                        routed_recoverable_txs.push(recovery_tx);
                        routed.rust_batch.push(tx, info);
                        self.metrics.batcher_routed_rust_total.add(1, &[]);
                    }
                    RouteDecision::Blockifier(reason) => {
                        if routed.blockifier_batch.len() >= self.routing_cfg.blockifier_batch_size {
                            cap_stop_branch = Some("blockifier");
                            deferred_txs.push(recovery_tx);
                            deferred_txs.extend(picked_iter);
                            break;
                        }
                        routed_recoverable_txs.push(recovery_tx);
                        routed.blockifier_batch.push(tx, info);
                        self.metrics.batcher_routed_blockifier_total.add(1, &[]);
                        let reason_label = match reason {
                            RouteFallbackReason::NotInvoke => "not_invoke",
                            RouteFallbackReason::SenderNotExecutor => "sender_not_executor",
                            RouteFallbackReason::SelectorNotSupported => "selector_not_supported",
                            RouteFallbackReason::ClassHashLookupFailed => "class_hash_lookup_failed",
                            RouteFallbackReason::ClassHashNotSupported => "class_hash_not_supported",
                            RouteFallbackReason::MulticallPartiallySupported => {
                                self.metrics.batcher_multicall_partial_supported_total.add(1, &[]);
                                "multicall_partially_supported"
                            }
                        };
                        self.metrics.batcher_route_fallback_total.add(1, &[KeyValue::new("reason", reason_label)]);
                    }
                },
            }
        }

        Ok(RoutedPickOutcome { routed, routed_recoverable_txs, deferred_txs, cap_stop_branch })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        backend: Arc<MadaraBackend>,
        mempool: Arc<Mempool>,
        l1_client: Arc<dyn SettlementClient>,
        ctx: ServiceContext,
        out: mpsc::Sender<RoutedBatchToExecute>,
        bypass_in: mpsc::Receiver<ValidatedTransaction>,
        mempool_intake_rx: watch::Receiver<MempoolIntakeMode>,
        mempool_intake_tx: watch::Sender<MempoolIntakeMode>,
        tainted_rebuild_active_rx: watch::Receiver<bool>,
        execution_mode_rx: watch::Receiver<ExecutionMode>,
        execution_epoch_rx: watch::Receiver<u64>,
        routing_cfg: RustExecRoutingConfig,
        metrics: Arc<BlockProductionMetrics>,
        replay_mode_enabled: bool,
    ) -> Self {
        Self {
            mempool,
            l1_message_stream: l1_client.create_message_to_l2_consumer(),
            ctx,
            out,
            bypass_in,
            mempool_intake_rx,
            mempool_intake_tx,
            tainted_rebuild_active_rx,
            execution_mode_rx,
            execution_epoch_rx,
            routing_cfg,
            metrics,
            batch_size: backend.chain_config().block_production_concurrency.batch_size,
            backend,
            replay_mode_enabled,
        }
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        let mut local_carry = VecDeque::new();
        loop {
            if *self.tainted_rebuild_active_rx.borrow() {
                tokio::select! {
                    _ = self.ctx.cancelled() => return anyhow::Ok(()),
                    res = self.tainted_rebuild_active_rx.changed() => {
                        if res.is_err() {
                            return anyhow::Ok(());
                        }
                        continue;
                    }
                }
            }

            // We use the permit API so that we don't have to remove transactions from the mempool until the last moment.
            // The buffer inside the channel is of size 1 - meaning we're preparing the next batch of transactions that will immediately be executed next, once
            // the worker has finished executing its current one.
            let permit_wait_started = Instant::now();
            let output_capacity_before_wait = self.out.capacity();
            let Some(Ok(permit)) = self.ctx.run_until_cancelled(self.out.reserve()).await else {
                // Stop condition: service stopped (ctx), or batch sender closed.
                return anyhow::Ok(());
            };
            let permit_wait_ms = permit_wait_started.elapsed().as_secs_f64() * 1000.0;
            if self.replay_mode_enabled {
                if permit_wait_ms >= BATCHER_OUTPUT_BACKPRESSURE_INFO_MS {
                    tracing::info!(
                        "batcher_output_backpressured permit_wait_ms={permit_wait_ms} output_capacity_before_wait={} output_capacity_after_reserve={}",
                        output_capacity_before_wait,
                        self.out.capacity()
                    );
                } else {
                    tracing::debug!(
                        "batcher_output_permit_acquired permit_wait_ms={permit_wait_ms} output_capacity_before_wait={} output_capacity_after_reserve={}",
                        output_capacity_before_wait,
                        self.out.capacity()
                    );
                }
            }

            let mempool_mode = *self.mempool_intake_rx.borrow();
            let flush_once = mempool_mode == MempoolIntakeMode::FlushOnce;
            if local_carry.is_empty() && mempool_mode == MempoolIntakeMode::FlushOnce && self.mempool.is_empty().await {
                let _ = self.mempool_intake_tx.send(MempoolIntakeMode::Paused);
                continue;
            }

            // Resolve the execution-frontier block number per design doc 02 §Replay-boundary rule:
            // use internal_preconfirmed_tip first (execution frontier), then confirmed_tip+1.
            let frontier_block_n = {
                let head_state = self.backend.chain_head_state();
                head_state
                    .internal_preconfirmed_tip
                    .or(head_state.confirmed_tip.and_then(|block_n| block_n.checked_add(1)))
                    .unwrap_or(0)
            };
            let execution_epoch = *self.execution_epoch_rx.borrow();
            let execution_mode = *self.execution_mode_rx.borrow();

            let mut chunk_size = self.pick_limit_for_mode(execution_mode);
            if self.replay_mode_enabled {
                if let Some(remaining) = self.backend.replay_boundary_remaining_dispatch_capacity(frontier_block_n) {
                    if remaining == 0 {
                        tokio::select! {
                            _ = self.ctx.cancelled() => return anyhow::Ok(()),
                            res = self.mempool_intake_rx.changed() => {
                                if res.is_err() {
                                    return anyhow::Ok(());
                                }
                            }
                            _ = tokio::time::sleep(Duration::from_millis(25)) => {}
                        }
                        continue;
                    }

                    chunk_size = chunk_size.min(remaining as usize);
                }
            }

            let picked = if !local_carry.is_empty() {
                drain_local_carry(&mut local_carry, chunk_size.max(1))
            } else {
                // We have 3 ingress streams:
                // * bypass inclusion (for admin rpc/chain bootstrapping purposes)
                // * l1 to l2 message transactions
                // * mempool transactions
                //
                // Local carry from a previous cycle is drained before polling these streams.
                let (chain_id, _sn_version) = (
                    self.backend.chain_config().chain_id.to_felt(),
                    self.backend.chain_config().latest_protocol_version,
                );

                let bypass_txs_stream =
                    stream::unfold(&mut self.bypass_in, |chan| async move { chan.recv().await.map(|tx| (tx, chan)) })
                        .map(Ok);

                let l1_txs_stream =
                    self.l1_message_stream.as_mut().map(|res| Ok(l1_to_recoverable_validated(res?, chain_id)));

                // Note: this is not hoisted out of the loop, because we don't want to keep the lock around when waiting on the output channel reserve().
                let mempool_txs_stream: BoxStream<'static, anyhow::Result<_>> = match mempool_mode {
                    MempoolIntakeMode::Paused => stream::pending().boxed(),
                    MempoolIntakeMode::Running | MempoolIntakeMode::FlushOnce => {
                        stream::unfold(self.mempool.clone(), |mempool| async move {
                            let consumer = mempool.get_consumer().await;
                            Some((consumer, mempool))
                        })
                        .map(|c| stream::iter(c.map(Ok)))
                        .flatten()
                        .boxed()
                    }
                };

                let tx_stream = stream::select_with_strategy(
                    bypass_txs_stream,
                    stream::select(l1_txs_stream, mempool_txs_stream), // round-bobbin strategy
                    |()| PollNext::Left, // always prioritise bypass_txs when there are ready items in multiple streams
                )
                .try_ready_chunks(chunk_size.max(1));

                tokio::pin!(tx_stream);

                tokio::select! {
                    _ = self.ctx.cancelled() => {
                        // Stop condition: cancelled.
                        return anyhow::Ok(());
                    }
                    res = self.mempool_intake_rx.changed() => {
                        if res.is_err() {
                            return anyhow::Ok(());
                        }
                        continue;
                    }
                    Some(got) = tx_stream.next() => {
                        // got a batch :)
                        let got = got.context("Creating batch for block building")?;
                        tracing::debug!("Batcher got a batch of {}.", got.len());
                        got
                    }
                    // Stop condition: tx_stream is empty.
                    else => return anyhow::Ok(())
                }
            };

            if !picked.is_empty() {
                let picked_total = picked.len();
                let picked_summary = summarize_validated_batch(&picked);
                let RoutedPickOutcome { routed, routed_recoverable_txs, deferred_txs, cap_stop_branch } =
                    self.route_picked_txs(picked, execution_mode, frontier_block_n, execution_epoch)?;
                let routed_total = routed.total_len();
                let current_epoch = *self.execution_epoch_rx.borrow();
                let carry_total_after = local_carry.len()
                    + deferred_txs.len()
                    + if current_epoch != execution_epoch { routed_recoverable_txs.len() } else { 0 };

                if let Some(branch) = cap_stop_branch {
                    tracing::debug!(
                        block_number = frontier_block_n,
                        mode = ?execution_mode,
                        branch,
                        picked_txs = picked_total,
                        routed_txs = routed_total,
                        deferred_txs = carry_total_after,
                        rust_cap = self.routing_cfg.rust_batch_size,
                        blockifier_cap = self.routing_cfg.blockifier_batch_size,
                        "batcher_branch_cap_reached"
                    );
                }

                self.metrics.batcher_cycles_total.add(1, &[]);
                self.metrics.batcher_picked_txs_total.add(picked_total as u64, &[]);
                self.metrics.batcher_output_rust_branch_size.record(routed.rust_batch.len() as f64, &[]);
                self.metrics.batcher_output_blockifier_branch_size.record(routed.blockifier_batch.len() as f64, &[]);

                tracing::info!(
                    block_number = frontier_block_n,
                    picked_txs = picked_total,
                    routed_txs = routed_total,
                    deferred_txs = carry_total_after,
                    blockifier_txs = routed.blockifier_batch.len(),
                    rust_txs = routed.rust_batch.len(),
                    mode = ?execution_mode,
                    "batcher_batch_routed"
                );

                if self.replay_mode_enabled {
                    let total = routed.total_len() as u64;
                    if let Some(status) = self.backend.replay_boundary_record_dispatched(frontier_block_n, total) {
                        if let Some(mismatch) = status.mismatch {
                            tracing::warn!(
                                "replay_boundary_mismatch_after_dispatch block_number={} expected_tx_count={} dispatched_tx_count={} executed_tx_count={} message={}",
                                frontier_block_n,
                                status.expected_tx_count,
                                status.dispatched_tx_count,
                                status.executed_tx_count,
                                mismatch
                            );
                        }
                    }
                }

                if current_epoch != execution_epoch {
                    tracing::warn!(
                        block_number = frontier_block_n,
                        picked_txs = picked_total,
                        picked_epoch = execution_epoch,
                        current_epoch,
                        first_tx_hash = picked_summary.first_hash.as_deref().unwrap_or("-"),
                        first_tx_nonce = picked_summary.first_nonce.as_deref().unwrap_or("-"),
                        last_tx_hash = picked_summary.last_hash.as_deref().unwrap_or("-"),
                        last_tx_nonce = picked_summary.last_nonce.as_deref().unwrap_or("-"),
                        "batcher_epoch_changed_before_send_recovering_picked_txs"
                    );
                    prepend_local_carry(
                        &mut local_carry,
                        routed_recoverable_txs.into_iter().chain(deferred_txs.into_iter()),
                    );
                    continue;
                }

                prepend_local_carry(&mut local_carry, deferred_txs);

                if routed.is_empty() {
                    continue;
                }

                let summary = summarize_validated_batch(&routed_recoverable_txs);
                tracing::debug!(
                    block_number = frontier_block_n,
                    picked_txs = picked_total,
                    routed_txs = routed_total,
                    deferred_txs = local_carry.len(),
                    execution_epoch,
                    first_tx_hash = summary.first_hash.as_deref().unwrap_or("-"),
                    first_tx_nonce = summary.first_nonce.as_deref().unwrap_or("-"),
                    last_tx_hash = summary.last_hash.as_deref().unwrap_or("-"),
                    last_tx_nonce = summary.last_nonce.as_deref().unwrap_or("-"),
                    "batcher_sending_routed_batch"
                );
                permit.send(routed);

                if flush_once && self.mempool.is_empty().await {
                    let _ = self.mempool_intake_tx.send(MempoolIntakeMode::Paused);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{stream, StreamExt};
    use mc_exec::execution::TxInfo;
    use mc_mempool::MempoolConfig;
    use mp_chain_config::ChainConfig;
    use mp_convert::ToFelt;
    use starknet_api::core::{ContractAddress, Nonce};
    use starknet_api::executable_transaction::{AccountTransaction, InvokeTransaction};
    use starknet_api::transaction::{InvokeTransaction as ApiInvokeTransaction, InvokeTransactionV3, TransactionHash};
    use starknet_types_core::felt::Felt;
    use tokio::time::{timeout, Duration};

    #[derive(Default)]
    struct EmptySettlementClient;

    impl SettlementClient for EmptySettlementClient {
        fn create_message_to_l2_consumer(&self) -> BoxStream<'static, anyhow::Result<L1HandlerTransactionWithFee>> {
            stream::pending().boxed()
        }
    }

    struct BatcherTestHarness {
        task: tokio::task::JoinHandle<anyhow::Result<()>>,
        cancel_ctx: ServiceContext,
        bypass_tx: mpsc::Sender<ValidatedTransaction>,
        out_rx: mpsc::Receiver<RoutedBatchToExecute>,
        mempool: Arc<Mempool>,
        _mempool_intake_tx: watch::Sender<MempoolIntakeMode>,
        _tainted_rebuild_tx: watch::Sender<bool>,
        _execution_mode_tx: watch::Sender<ExecutionMode>,
        _execution_epoch_tx: watch::Sender<u64>,
    }

    fn validated_invoke(sender_address: Felt, nonce: u64, hash: u64) -> ValidatedTransaction {
        ValidatedTransaction::from_starknet_api(
            AccountTransaction::Invoke(InvokeTransaction {
                tx: ApiInvokeTransaction::V3(InvokeTransactionV3 {
                    sender_address: ContractAddress::try_from(sender_address).expect("valid sender address"),
                    resource_bounds: Default::default(),
                    tip: Default::default(),
                    signature: Default::default(),
                    nonce: Nonce(Felt::from(nonce)),
                    calldata: Default::default(),
                    nonce_data_availability_mode: Default::default(),
                    fee_data_availability_mode: Default::default(),
                    paymaster_data: Default::default(),
                    account_deployment_data: Default::default(),
                }),
                tx_hash: TransactionHash(Felt::from(hash)),
            }),
            TxTimestamp::now(),
            None,
            true,
        )
    }

    fn batch_hashes(batch: &RoutedBatchToExecute) -> Vec<Felt> {
        batch.blockifier_batch.txs.iter().chain(batch.rust_batch.txs.iter()).map(|tx| tx.tx_hash().to_felt()).collect()
    }

    fn spawn_batcher_harness(initial_mode: ExecutionMode, routing_cfg: RustExecRoutingConfig) -> BatcherTestHarness {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        let mempool = Arc::new(Mempool::new(
            backend.clone(),
            MempoolConfig::default().with_save_to_db(false).with_load_from_db(false),
        ));
        let (out_tx, out_rx) = mpsc::channel(4);
        let (bypass_tx, bypass_rx) = mpsc::channel(8);
        let (mempool_intake_tx, mempool_intake_rx) = watch::channel(MempoolIntakeMode::Paused);
        let (tainted_rebuild_tx, tainted_rebuild_rx) = watch::channel(false);
        let (execution_mode_tx, execution_mode_rx) = watch::channel(initial_mode);
        let (execution_epoch_tx, execution_epoch_rx) = watch::channel(0u64);
        let ctx = ServiceContext::new_for_testing();
        let cancel_ctx = ctx.clone();

        let batcher = Batcher::new(
            backend,
            mempool.clone(),
            Arc::new(EmptySettlementClient),
            ctx,
            out_tx,
            bypass_rx,
            mempool_intake_rx,
            mempool_intake_tx.clone(),
            tainted_rebuild_rx,
            execution_mode_rx,
            execution_epoch_rx,
            routing_cfg,
            Arc::new(BlockProductionMetrics::register()),
            false,
        );
        let task = tokio::spawn(async move { batcher.run().await });

        BatcherTestHarness {
            task,
            cancel_ctx,
            bypass_tx,
            out_rx,
            mempool,
            _mempool_intake_tx: mempool_intake_tx,
            _tainted_rebuild_tx: tainted_rebuild_tx,
            _execution_mode_tx: execution_mode_tx,
            _execution_epoch_tx: execution_epoch_tx,
        }
    }

    #[tokio::test]
    async fn blockifier_only_pick_limit_keeps_bypass_suffix_out_of_mempool() {
        let mut routing_cfg = RustExecRoutingConfig::default();
        routing_cfg.blockifier_batch_size = 1;
        routing_cfg.rust_batch_size = 1;

        let mut harness = spawn_batcher_harness(ExecutionMode::BlockifierOnly, routing_cfg);
        let tx1 = validated_invoke(Felt::from(0x111u64), 0, 0xaaa1);
        let tx2 = validated_invoke(Felt::from(0x111u64), 1, 0xaaa2);

        harness.bypass_tx.send(tx1.clone()).await.expect("enqueue first bypass tx");
        harness.bypass_tx.send(tx2.clone()).await.expect("enqueue second bypass tx");

        let first_batch = timeout(Duration::from_secs(2), harness.out_rx.recv())
            .await
            .expect("first routed batch timeout")
            .expect("batcher output open");
        assert_eq!(batch_hashes(&first_batch), vec![tx1.hash]);
        assert!(harness.mempool.is_empty().await, "BlockifierOnly spillback must stay out of mempool");

        let second_batch = timeout(Duration::from_secs(2), harness.out_rx.recv())
            .await
            .expect("second routed batch timeout")
            .expect("batcher output open");
        assert_eq!(batch_hashes(&second_batch), vec![tx2.hash]);
        assert!(harness.mempool.is_empty().await, "suffix should remain outside mempool across cycles");

        harness.cancel_ctx.cancel_global();
        timeout(Duration::from_secs(2), async { harness.task.await.expect("batcher task join") })
            .await
            .expect("batcher shutdown timeout")
            .expect("batcher task result");
    }

    #[tokio::test]
    async fn local_carry_retries_before_new_bypass_traffic() {
        let mut routing_cfg = RustExecRoutingConfig::default();
        routing_cfg.blockifier_batch_size = 1;
        routing_cfg.rust_batch_size = 1;

        let mut harness = spawn_batcher_harness(ExecutionMode::Mixed, routing_cfg);
        let tx1 = validated_invoke(Felt::from(0x222u64), 0, 0xbbb1);
        let tx2 = validated_invoke(Felt::from(0x222u64), 1, 0xbbb2);
        let tx3 = validated_invoke(Felt::from(0x222u64), 2, 0xbbb3);

        harness.bypass_tx.send(tx1.clone()).await.expect("enqueue first bypass tx");
        harness.bypass_tx.send(tx2.clone()).await.expect("enqueue second bypass tx");

        let first_batch = timeout(Duration::from_secs(2), harness.out_rx.recv())
            .await
            .expect("first routed batch timeout")
            .expect("batcher output open");
        assert_eq!(batch_hashes(&first_batch), vec![tx1.hash]);
        assert!(harness.mempool.is_empty().await, "carried suffix must remain local to the batcher");

        harness.bypass_tx.send(tx3.clone()).await.expect("enqueue third bypass tx");

        let second_batch = timeout(Duration::from_secs(2), harness.out_rx.recv())
            .await
            .expect("second routed batch timeout")
            .expect("batcher output open");
        assert_eq!(batch_hashes(&second_batch), vec![tx2.hash], "local carry must win before new bypass ingress");

        let third_batch = timeout(Duration::from_secs(2), harness.out_rx.recv())
            .await
            .expect("third routed batch timeout")
            .expect("batcher output open");
        assert_eq!(
            batch_hashes(&third_batch),
            vec![tx3.hash],
            "new bypass traffic should arrive after the drained carry"
        );
        assert!(harness.mempool.is_empty().await, "retrying a carried suffix must not route through mempool");

        harness.cancel_ctx.cancel_global();
        timeout(Duration::from_secs(2), async { harness.task.await.expect("batcher task join") })
            .await
            .expect("batcher shutdown timeout")
            .expect("batcher task result");
    }

    #[test]
    fn epoch_change_recovery_prepends_picked_txs_ahead_of_existing_local_carry() {
        let tx1 = validated_invoke(Felt::from(0x333u64), 0, 0xccc1);
        let tx2 = validated_invoke(Felt::from(0x333u64), 1, 0xccc2);
        let tx3 = validated_invoke(Felt::from(0x333u64), 2, 0xccc3);
        let mut local_carry = VecDeque::from([tx3.clone()]);

        prepend_local_carry(&mut local_carry, vec![tx1.clone(), tx2.clone()]);

        let replayed = drain_local_carry(&mut local_carry, 3);
        let replayed_hashes = replayed.into_iter().map(|tx| tx.hash).collect::<Vec<_>>();
        assert_eq!(
            replayed_hashes,
            vec![tx1.hash, tx2.hash, tx3.hash],
            "stale-epoch recovery must keep already-picked txs local and ahead of older untouched carry",
        );
    }
}
