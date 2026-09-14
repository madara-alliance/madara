use super::*;
use futures::FutureExt;
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
