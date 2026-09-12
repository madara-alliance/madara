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
        std::iter::from_fn(|| batch.next_contiguous()).take(16).map(|tx| tx.transaction.nonce()).collect();
    assert_eq!(nonces, (0u64..16).map(Felt::from).collect::<Vec<_>>());
    assert_eq!(*batch.lock.account_nonces().next().unwrap().1, Nonce(Felt::ZERO));
    batch.lock.check_invariants();
    drop(batch);
    assert!(consumer(&pool).now_or_never().is_none(), "a new batch must not inherit a speculative cursor");
    pool.write().await.update_account_nonce(&Felt::ONE.try_into().unwrap(), &Nonce(Felt::from(16u64)), &mut vec![]);
    let mut batch = consumer(&pool).await;
    assert_eq!(std::iter::from_fn(|| batch.next_contiguous()).count(), 4);
    batch.lock.check_invariants();
}

#[tokio::test]
async fn contiguous_batch_stops_at_gap_and_serves_another_account() {
    let pool = MempoolInnerWithNotify::new(&ChainConfig::madara_test());
    queue(&pool, 1, [0, 1, 3]).await;
    queue(&pool, 2, [0, 1]).await;
    let mut batch = consumer(&pool).await;
    let txs: Vec<_> = std::iter::from_fn(|| batch.next_contiguous()).collect();
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
    let mut txs: Vec<_> = std::iter::from_fn(|| batch.next_contiguous()).collect();
    drop(batch);
    // Nonce 0 is rejected (not reverted). Return only its still-future successors.
    for tx in txs.drain(1..) {
        pool.write().await.insert_tx(TxTimestamp::now(), tx, Nonce(Felt::ZERO), &mut vec![]).unwrap();
    }
    assert!(consumer(&pool).now_or_never().is_none());
    queue(&pool, 1, [0]).await;
    let mut batch = consumer(&pool).await;
    assert_eq!(
        std::iter::from_fn(|| batch.next_contiguous()).map(|tx| tx.transaction.nonce()).collect::<Vec<_>>(),
        (0u64..4).map(Felt::from).collect::<Vec<_>>()
    );
    batch.lock.check_invariants();
}

#[tokio::test]
async fn dropped_partial_consumer_keeps_unconsumed_successors() {
    let pool = MempoolInnerWithNotify::new(&ChainConfig::madara_test());
    queue(&pool, 1, 0..4).await;
    let mut batch = consumer(&pool).await;
    assert_eq!(batch.next_contiguous().unwrap().transaction.nonce(), Felt::ZERO);
    drop(batch);
    let lock = pool.read().await;
    assert_eq!(lock.transactions_by_arrival().count(), 3);
    assert_eq!(*lock.account_nonces().next().unwrap().1, Nonce(Felt::ZERO));
}
