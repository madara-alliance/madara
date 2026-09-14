use super::*;
use futures::FutureExt;
use mc_exec::execution::TxInfo;
use mc_mempool::MempoolConfig;
use mc_settlement_client::L1ClientMock;
use mp_chain_config::{BlockProductionConfig, ChainConfig, MempoolMode};
use mp_convert::Felt;
use mp_transactions::{InvokeTransaction, InvokeTransactionV3, Transaction};
use std::collections::BTreeMap;

#[rstest::rstest]
#[case::default_keeps_head_only_behavior(3, 1024, None, 3)]
#[case::single_account(1, 1024, Some(300), 300)]
#[case::three_accounts(3, 1024, Some(300), 900)]
#[case::full_batch(4, 1024, Some(300), 1024)]
#[case::smaller_batch(3, 128, Some(300), 128)]
#[case::configured_limit(3, 1024, Some(7), 21)]
#[case::zero_limit_keeps_progress(3, 1024, Some(0), 3)]
#[tokio::test]
async fn next_batch_limits_each_account_and_keeps_remaining_transactions(
    #[case] n_accounts: u64,
    #[case] batch_size: usize,
    #[case] configured_limit: Option<usize>,
    #[case] expected_len: usize,
    #[values(MempoolMode::Timestamp, MempoolMode::Tip)] mode: MempoolMode,
) {
    let mut block_production_concurrency = BlockProductionConfig { batch_size, ..Default::default() };
    if let Some(limit) = configured_limit {
        block_production_concurrency.max_txs_per_account_per_batch = limit;
    }
    let max_txs_per_account_per_batch = block_production_concurrency.max_txs_per_account_per_batch;
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig {
        block_production_concurrency,
        mempool_mode: mode,
        ..ChainConfig::madara_test()
    }));
    let mempool = Arc::new(Mempool::new(backend.clone(), MempoolConfig::default().with_save_to_db(false)));
    for account in 1..=n_accounts {
        for nonce in 0u64..500 {
            mempool
                .accept_tx(ValidatedTransaction {
                    transaction: Transaction::Invoke(InvokeTransaction::V3(InvokeTransactionV3 {
                        sender_address: Felt::from(account),
                        nonce: Felt::from(nonce),
                        tip: if account == 1 && nonce == 0 { 100 } else { 10 },
                        ..Default::default()
                    })),
                    paid_fee_on_l1: None,
                    contract_address: Felt::from(account),
                    arrived_at: TxTimestamp::now(),
                    declared_class: None,
                    hash: Felt::from(account * 1000 + nonce),
                    charge_fee: true,
                })
                .await
                .unwrap();
        }
    }
    let (out, _out_rx) = mpsc::channel(1);
    let (_bypass_tx, bypass_in) = mpsc::channel(1);
    let (_intake_tx, mempool_intake_rx) = watch::channel(MempoolIntakeMode::Running);
    let l1_client = Arc::new(L1ClientMock::new());
    let mut batcher = Batcher::new(
        backend,
        mempool.clone(),
        Arc::new(BlockProductionMetrics::register()),
        l1_client.clone(),
        ServiceContext::new_for_testing(),
        out,
        bypass_in,
        mempool_intake_rx,
    );
    let step = tokio::time::timeout(std::time::Duration::from_secs(5), batcher.next_batch()).await.unwrap().unwrap();
    let BatcherStep::Batch(batch) = step else { panic!("expected a transaction batch") };
    assert_eq!(batch.len(), expected_len);
    assert!(batch.additional_info.iter().all(|info| info.from_mempool));

    let mut taken = BTreeMap::<Felt, Vec<Felt>>::new();
    for tx in batch.txs {
        let blockifier::transaction::transaction_execution::Transaction::Account(tx) = tx else {
            panic!("expected an account transaction")
        };
        taken.entry(tx.tx.contract_address().to_felt()).or_default().push(tx.tx.nonce().to_felt());
    }
    for nonces in taken.values() {
        assert!(nonces.len() <= max_txs_per_account_per_batch.max(1));
        assert_eq!(*nonces, (0..nonces.len() as u64).map(Felt::from).collect::<Vec<_>>());
    }

    let remaining = mempool.snapshot_transactions_matching(0, usize::MAX, false, |_| true).await;
    assert_eq!(remaining.len(), n_accounts as usize * 500 - expected_len);
    let mut queued = BTreeMap::<Felt, Vec<Felt>>::new();
    for entry in remaining {
        queued.entry(entry.transaction.contract_address).or_default().push(entry.transaction.transaction.nonce());
    }
    for account in 1..=n_accounts {
        let address = Felt::from(account);
        let start = taken.get(&address).map_or(0, |nonces| nonces.len()) as u64;
        let nonces = queued.get_mut(&address).unwrap();
        nonces.sort();
        assert_eq!(*nonces, (start..500).map(Felt::from).collect::<Vec<_>>());
    }

    if expected_len == n_accounts as usize * max_txs_per_account_per_batch.max(1) {
        // All heads are in flight: another batch must wait for real nonce progress.
        assert!(batcher.next_batch().now_or_never().is_none());
    }
}

struct Harness {
    batcher: Batcher,
    mempool: Arc<Mempool>,
    output: mpsc::Receiver<BatchToExecute>,
    intake: watch::Sender<MempoolIntakeMode>,
    ctx: ServiceContext,
    _bypass: mpsc::Sender<ValidatedTransaction>,
    _l1: Arc<L1ClientMock>,
}

impl Harness {
    fn new() -> Self {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig {
            block_production_concurrency: BlockProductionConfig {
                batch_size: 100,
                max_txs_per_account_per_batch: 30,
                ..Default::default()
            },
            ..ChainConfig::madara_test()
        }));
        let mempool = Arc::new(Mempool::new(backend.clone(), MempoolConfig::default().with_save_to_db(false)));
        let (out, output) = mpsc::channel(1);
        let (bypass, bypass_in) = mpsc::channel(1);
        let (intake, intake_rx) = watch::channel(MempoolIntakeMode::Running);
        let l1 = Arc::new(L1ClientMock::new());
        let ctx = ServiceContext::new_for_testing();
        let batcher = Batcher::new(
            backend,
            mempool.clone(),
            Arc::new(BlockProductionMetrics::register()),
            l1.clone(),
            ctx.clone(),
            out,
            bypass_in,
            intake_rx,
        );
        Self { batcher, mempool, output, intake, ctx, _bypass: bypass, _l1: l1 }
    }
}

fn queued_tx(account: u64, nonce: u64) -> ValidatedTransaction {
    ValidatedTransaction {
        transaction: Transaction::Invoke(InvokeTransaction::V3(InvokeTransactionV3 {
            sender_address: Felt::from(account),
            nonce: Felt::from(nonce),
            ..Default::default()
        })),
        contract_address: Felt::from(account),
        hash: Felt::from(account * 1000 + nonce),
        arrived_at: TxTimestamp::now(),
        paid_fee_on_l1: None,
        declared_class: None,
        charge_fee: true,
    }
}

#[tokio::test]
async fn cancellation_under_output_backpressure_keeps_transactions_and_releases_lock() {
    let mut h = Harness::new();
    for nonce in 0..40 {
        h.mempool.accept_tx(queued_tx(1, nonce)).await.unwrap();
    }
    h.batcher.out.send(BatchToExecute::default()).await.unwrap();
    let mut running = Box::pin(h.batcher.run());
    assert!(running.as_mut().now_or_never().is_none());
    h.ctx.cancel_global();
    tokio::time::timeout(std::time::Duration::from_secs(5), running).await.unwrap().unwrap();
    let queued = h.mempool.snapshot_transactions_matching(0, usize::MAX, false, |_| true).await;
    assert_eq!(queued.len(), 40);
    assert!(h.output.recv().await.unwrap().is_empty());
    assert!(h.output.recv().await.is_none());
    h.mempool.accept_tx(queued_tx(2, 0)).await.unwrap();
    assert_eq!(h.mempool.get_consumer().await.count(), 2);
}

#[tokio::test]
async fn pause_resume_rebuilds_waiting_stream_without_losing_transactions() {
    let mut h = Harness::new();
    let mut waiting = Box::pin(h.batcher.next_batch());
    assert!(waiting.as_mut().now_or_never().is_none());
    h.intake.send(MempoolIntakeMode::Paused).unwrap();
    assert!(matches!(waiting.await.unwrap(), BatcherStep::RebuildStreams));
    for nonce in 0..40 {
        h.mempool.accept_tx(queued_tx(1, nonce)).await.unwrap();
    }
    let mut paused = Box::pin(h.batcher.next_batch());
    assert!(paused.as_mut().now_or_never().is_none());
    h.intake.send(MempoolIntakeMode::Running).unwrap();
    assert!(matches!(paused.await.unwrap(), BatcherStep::RebuildStreams));
    let BatcherStep::Batch(batch) = h.batcher.next_batch().await.unwrap() else { panic!("expected batch") };
    assert_eq!(
        batch.txs.iter().map(|tx| tx.tx_hash().to_felt()).collect::<Vec<_>>(),
        (0..30).map(|nonce| queued_tx(1, nonce).hash).collect::<Vec<_>>()
    );
    assert_eq!(h.mempool.snapshot_transactions_matching(0, usize::MAX, false, |_| true).await.len(), 10);
}

#[tokio::test]
async fn cancelling_idle_consumer_does_not_lose_next_admission_wakeup() {
    let mut h = Harness::new();
    let mut waiting = Box::pin(h.batcher.next_batch());
    assert!(waiting.as_mut().now_or_never().is_none());
    h.ctx.cancel_global();
    assert!(matches!(waiting.await.unwrap(), BatcherStep::Stop));
    h.mempool.accept_tx(queued_tx(1, 0)).await.unwrap();
    let mut consumer = tokio::time::timeout(std::time::Duration::from_secs(5), h.mempool.get_consumer()).await.unwrap();
    assert_eq!(consumer.next().unwrap().hash, queued_tx(1, 0).hash);
}
