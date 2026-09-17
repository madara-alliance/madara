//! Batch-local nonce cursor regressions. No executed nonce is speculatively committed.
use super::*;
use futures::FutureExt;
use mp_chain_config::ChainConfig;
use mp_transactions::{InvokeTransaction, Transaction};

async fn queue(pool: &MempoolInnerWithNotify, address: u64, nonces: impl IntoIterator<Item = u64>) {
    for nonce in nonces {
        let mut tx = tests::tx_account(Felt::from(address));
        let Transaction::Invoke(InvokeTransaction::V3(inner)) = &mut tx.transaction else { unreachable!() };
        inner.nonce = Felt::from(nonce);
        pool.write().await.insert_tx(TxTimestamp::now(), tx, Nonce(Felt::ZERO), &mut vec![]).unwrap();
    }
}

async fn consumer(pool: &MempoolInnerWithNotify) -> MempoolConsumer {
    MempoolConsumer { lock: pool.get_write_access_wait_for_ready().await, successor: None }
}

#[tokio::test]
async fn contiguous_batch_stops_at_cap_and_waits_for_executed_nonce() {
    let pool = MempoolInnerWithNotify::new(&ChainConfig::madara_test());
    queue(&pool, 1, 0..20).await;
    let mut batch = consumer(&pool).await;
    let nonces: Vec<_> =
        std::iter::from_fn(|| batch.next_contiguous(300)).take(16).map(|tx| tx.transaction.nonce()).collect();
    assert_eq!(nonces, (0u64..16).map(Felt::from).collect::<Vec<_>>());
    assert_eq!(*batch.lock.account_nonces().next().unwrap().1, Nonce(Felt::ZERO));
    batch.lock.check_invariants();
    drop(batch);
    assert!(consumer(&pool).now_or_never().is_none(), "a new batch must not inherit a speculative cursor");
    pool.write().await.update_account_nonce(&Felt::ONE.try_into().unwrap(), &Nonce(Felt::from(16u64)), &mut vec![]);
    let mut batch = consumer(&pool).await;
    assert_eq!(std::iter::from_fn(|| batch.next_contiguous(300)).count(), 4);
    batch.lock.check_invariants();
}

#[tokio::test]
async fn contiguous_batch_stops_at_gap_and_serves_another_account() {
    let pool = MempoolInnerWithNotify::new(&ChainConfig::madara_test());
    queue(&pool, 1, [0, 1, 3]).await;
    queue(&pool, 2, [0, 1]).await;
    let mut batch = consumer(&pool).await;
    let txs: Vec<_> = std::iter::from_fn(|| batch.next_contiguous(300)).collect();
    let mut actual: Vec<_> = txs.iter().map(|tx| (tx.contract_address, tx.transaction.nonce())).collect();
    actual.sort(); // Equal arrival timestamps may select either account first.
    assert_eq!(actual, [(1u64, 0u64), (1, 1), (2, 0), (2, 1)].map(|(a, n)| (Felt::from(a), Felt::from(n))));
    assert!(batch.lock.get_transaction(&Felt::ONE.try_into().unwrap(), &Nonce(Felt::from(3u64))).is_some());
    batch.lock.check_invariants();
}

#[tokio::test]
async fn ordinary_consumer_still_returns_only_ready_heads() {
    let pool = MempoolInnerWithNotify::new(&ChainConfig::madara_test());
    queue(&pool, 1, 0..20).await;
    assert_eq!(consumer(&pool).await.count(), 1);
}

#[tokio::test]
async fn deferred_successors_wait_for_replacement_of_rejected_head() {
    let pool = MempoolInnerWithNotify::new(&ChainConfig::madara_test());
    queue(&pool, 1, 0..4).await;
    let mut batch = consumer(&pool).await;
    let mut txs: Vec<_> = std::iter::from_fn(|| batch.next_contiguous(300)).collect();
    drop(batch);
    // Nonce 0 is rejected (not reverted). Return only its still-future successors.
    for tx in txs.drain(1..) {
        let mut lock = pool.write().await;
        lock.release_consumed(&tx.hash);
        lock.insert_tx(TxTimestamp::now(), tx, Nonce(Felt::ZERO), &mut vec![]).unwrap();
    }
    pool.write().await.release_consumed(&txs[0].hash);
    assert!(consumer(&pool).now_or_never().is_none());
    queue(&pool, 1, [0]).await;
    let mut batch = consumer(&pool).await;
    assert_eq!(
        std::iter::from_fn(|| batch.next_contiguous(300)).map(|tx| tx.transaction.nonce()).collect::<Vec<_>>(),
        (0u64..4).map(Felt::from).collect::<Vec<_>>()
    );
    batch.lock.check_invariants();
}

#[tokio::test]
async fn dropped_partial_consumer_keeps_unconsumed_successors() {
    let pool = MempoolInnerWithNotify::new(&ChainConfig::madara_test());
    queue(&pool, 1, 0..4).await;
    let mut batch = consumer(&pool).await;
    assert_eq!(batch.next_contiguous(300).unwrap().transaction.nonce(), Felt::ZERO);
    drop(batch);
    let lock = pool.read().await;
    assert_eq!(lock.transactions_by_arrival().count(), 3);
    assert_eq!(*lock.account_nonces().next().unwrap().1, Nonce(Felt::ZERO));
}

#[tokio::test]
async fn account_cap_keeps_successors_until_nonce_progress_and_resets_for_next_batch() {
    let pool = MempoolInnerWithNotify::new(&ChainConfig::madara_test());
    queue(&pool, 1, 0..650).await;
    let address = Felt::ONE.try_into().unwrap();

    for (start, end) in [(0u64, 300u64), (300, 600), (600, 650)] {
        if start > 0 {
            assert!(consumer(&pool).now_or_never().is_none());
            pool.write().await.update_account_nonce(&address, &Nonce(Felt::from(start)), &mut vec![]);
        }
        let mut batch = consumer(&pool).await;
        let nonces: Vec<_> =
            std::iter::from_fn(|| batch.next_contiguous(300)).map(|tx| tx.transaction.nonce()).collect();
        assert_eq!(nonces, (start..end).map(Felt::from).collect::<Vec<_>>());
        assert_eq!(batch.lock.transactions_by_arrival().count(), (650 - end) as usize);
        batch.lock.check_invariants();
    }
}

#[rstest::rstest]
#[tokio::test]
async fn in_flight_capacity_preserves_deferred_work_against_new_arrivals(
    #[values(mp_chain_config::MempoolFullPolicy::RejectNew, mp_chain_config::MempoolFullPolicy::EvictLessDesirable)]
    full_policy: mp_chain_config::MempoolFullPolicy,
    #[values(mp_chain_config::MempoolMode::Timestamp, mp_chain_config::MempoolMode::Tip)]
    mode: mp_chain_config::MempoolMode,
) {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig {
        mempool_max_transactions: 3,
        mempool_full_policy: full_policy,
        mempool_mode: mode,
        ..ChainConfig::madara_test()
    }));
    let mempool = Mempool::new(backend, MempoolConfig::default());
    let make_tx = |account: u64, nonce: u64| {
        let mut tx = tests::tx_account(Felt::from(account));
        let Transaction::Invoke(InvokeTransaction::V3(inner)) = &mut tx.transaction else { unreachable!() };
        inner.nonce = Felt::from(nonce);
        inner.tip = if account == 1 { 0 } else { 100 };
        tx.hash = Felt::from(account * 1000 + nonce);
        tx
    };
    for nonce in 0..3 {
        mempool.accept_tx(make_tx(1, nonce)).await.unwrap();
    }
    let mut consumer = mempool.get_consumer().await;
    let removed: Vec<_> = std::iter::from_fn(|| consumer.next_contiguous(300)).collect();
    drop(consumer);
    for account in 2..5 {
        assert!(matches!(
            mempool.accept_tx(make_tx(account, 0)).await,
            Err(MempoolInsertionError::InnerMempool(inner::TxInsertionError::Limit(_)))
        ));
    }
    assert!(matches!(
        mempool.accept_tx(make_tx(1, 1)).await,
        Err(MempoolInsertionError::InnerMempool(inner::TxInsertionError::DuplicateTxn))
    ));
    // Only the rejected head releases a slot. A new arrival can use that slot,
    // but the still-future successors retain theirs until requeue transfers them.
    mempool.finish_consumed_transactions(&[make_tx(1, 0).hash]).await;
    mempool.accept_tx(make_tx(2, 0)).await.unwrap();
    let mut notifications = mempool.subscribe_new_transactions();
    for tx in removed.into_iter().skip(1) {
        mempool.requeue_tx(tx).await.unwrap();
    }
    let mut hashes: Vec<_> = mempool
        .snapshot_transactions_matching(0, usize::MAX, false, |_| true)
        .await
        .into_iter()
        .map(|entry| entry.transaction.hash)
        .collect();
    hashes.sort();
    assert_eq!(hashes, [1001u64, 1002, 2000].map(Felt::from));
    assert!(matches!(notifications.try_recv(), Err(tokio::sync::broadcast::error::TryRecvError::Empty)));
    mempool.inner.read().await.check_invariants();
    // Restoring pending successors does not make the rejected head ready.
    let heads: Vec<_> = mempool.get_consumer().await.map(|tx| tx.hash).collect();
    assert_eq!(heads.len(), 1);
    assert!(!heads.contains(&Felt::from(1001u64)));
    assert!(mempool.get_consumer().now_or_never().is_none());
}
