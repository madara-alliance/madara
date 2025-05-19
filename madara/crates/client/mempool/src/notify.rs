use crate::{MempoolInner, MempoolLimits, MempoolTransaction, TxInsertionError};
use mc_db::mempool_db::NonceInfo;
use mp_convert::{Felt, ToFelt};
use starknet_api::core::Nonce;
use std::collections::BTreeMap;
use tokio::sync::{Notify, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// A view into the mempool, intended for consuming transactions. This is expected to be used by block production to
/// pop transactions from the mempool and execute them.
///
/// This struct implements [`Iterator`] by popping the next transaction to execute from the mempool.
///
/// This holds the lock to the inner mempool - use with care.
pub struct MempoolConsumerView<'a> {
    notify: &'a Notify,
    inner: RwLockWriteGuard<'a, MempoolInner>,
    nonce_cache: RwLockWriteGuard<'a, BTreeMap<Felt, Nonce>>,
}
impl Iterator for MempoolConsumerView<'_> {
    type Item = MempoolTransaction;
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.pop_next().inspect(|tx| {
            let contract_address = tx.contract_address().to_felt();
            let nonce_next = tx.nonce_next;
            self.nonce_cache.insert(contract_address, nonce_next);
        })
    }
}
impl MempoolConsumerView<'_> {
    /// Total number of transactions in the mempool. This does not mean they all are currently executable.
    pub fn n_txs_total(&self) -> usize {
        self.inner.n_total()
    }
}

impl Drop for MempoolConsumerView<'_> {
    fn drop(&mut self) {
        // If there are still ready transactions in the mempool, notify the next waiter.
        if self.inner.has_ready_transactions() {
            tracing::debug!("notify_one (drop)");
            self.notify.notify_one();
        }
    }
}

// Mutex Ordering (deadlocks): nonce_cache mutex should always be taken before inner mutex when the two are taken at the same time.
pub(crate) struct MempoolInnerWithNotify {
    inner: RwLock<MempoolInner>,
    // TODO: should this be in inner mempool? I don't understand why it's here.
    nonce_cache: RwLock<BTreeMap<Felt, Nonce>>,
    // Notify listener when the mempool goes from !has_ready_transactions to has_ready_transactions.
    notify: Notify,
}
impl MempoolInnerWithNotify {
    pub fn new(limits: MempoolLimits) -> Self {
        Self {
            inner: RwLock::new(MempoolInner::new(limits)),
            nonce_cache: Default::default(),
            notify: Default::default(),
        }
    }

    /// Insert a transaction into the inner mempool, possibly waking a waiting consumer.
    pub async fn insert_tx(
        &self,
        mempool_tx: MempoolTransaction,
        force: bool,
        update_limits: bool,
        nonce_info: NonceInfo,
    ) -> Result<(), TxInsertionError> {
        let mut lock = self.inner.write().await;
        lock.insert_tx(mempool_tx, force, update_limits, nonce_info)?; // On insert error, bubble up and do not notify.

        if lock.has_ready_transactions() {
            // We notify a single waiter. The waked task is in charge of waking the next waker in the notify if there are still transactions
            // in the mempool after it's done.
            tracing::debug!("notify_one (insert)");
            self.notify.notify_one();
        }

        Ok(())
    }

    /// Returns a reading view of the inner mempool.
    pub async fn read(&self) -> RwLockReadGuard<'_, MempoolInner> {
        self.inner.read().await
    }

    pub async fn nonce_cache_read(&self) -> RwLockReadGuard<'_, BTreeMap<Felt, Nonce>> {
        self.nonce_cache.read().await
    }

    #[cfg(test)]
    pub async fn write(&self) -> RwLockWriteGuard<'_, MempoolInner> {
        self.inner.write().await
    }
    #[cfg(test)]
    pub async fn nonce_cache_write(&self) -> RwLockWriteGuard<'_, BTreeMap<Felt, Nonce>> {
        self.nonce_cache.write().await
    }

    /// Returns a view of the mempool intended for consuming transactions from the mempool.
    /// If the mempool has no transaction that can be consumed, this function will wait until there is at least 1 transaction to consume.
    pub async fn get_consumer_wait_for_ready_tx(&self) -> MempoolConsumerView<'_> {
        let permit = self.notify.notified(); // This doesn't actually register us to be notified yet.
        tokio::pin!(permit);
        loop {
            {
                tracing::debug!("taking lock");
                let nonce_cache = self.nonce_cache.write().await;
                let inner = self.inner.write().await;

                if inner.has_ready_transactions() {
                    tracing::debug!("consumer ready");
                    return MempoolConsumerView { inner, nonce_cache, notify: &self.notify };
                }
                // Note: we put ourselves in the notify list BEFORE giving back the lock.
                // Otherwise, some transactions could be missed.
                permit.as_mut().enable(); // Register us to be notified.

                // drop the locks here
            }
            tracing::debug!("waiting");
            permit.as_mut().await; // Wait until we're notified.
            permit.set(self.notify.notified());
        }
    }

    /// Returns a view of the mempool intended for consuming transactions from the mempool.
    pub async fn get_consumer(&self) -> MempoolConsumerView<'_> {
        MempoolConsumerView {
            notify: &self.notify,
            nonce_cache: self.nonce_cache.write().await,
            inner: self.inner.write().await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::tx_account_v0_valid;
    use futures::FutureExt;
    use mp_transactions::validated::{TxTimestamp, ValidatedMempoolTx};
    use std::sync::Arc;

    #[rstest::rstest]
    #[tokio::test]
    async fn test_mempool_notify(tx_account_v0_valid: ValidatedMempoolTx) {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer()
            .try_init();
        let mempool = Arc::new(MempoolInnerWithNotify::new(MempoolLimits::for_testing()));

        let mut fut = Box::pin(mempool.get_consumer_wait_for_ready_tx());

        // poll once
        assert!(fut.as_mut().now_or_never().is_none());

        let nonce_info = NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE));
        let mempool_tx = MempoolTransaction {
            tx: tx_account_v0_valid.into_blockifier().unwrap().0,
            arrived_at: TxTimestamp::now(),
            converted_class: None,
            nonce: nonce_info.nonce,
            nonce_next: nonce_info.nonce_next,
        };
        mempool.insert_tx(mempool_tx.clone(), false, true, nonce_info).await.unwrap();

        // poll once
        let mut consumer = fut.as_mut().now_or_never().unwrap();
        let received = consumer.next().unwrap();
        assert_eq!(received.contract_address(), mempool_tx.contract_address());
        assert_eq!(received.nonce(), mempool_tx.nonce());
        assert_eq!(received.tx_hash(), mempool_tx.tx_hash());
    }

    #[rstest::rstest]
    #[tokio::test]
    /// This is unused as of yet in madara, but the mempool supports having multiple waiters on the notify.
    async fn test_mempool_notify_multiple_listeners(tx_account_v0_valid: ValidatedMempoolTx) {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer()
            .try_init();
        let mempool = Arc::new(MempoolInnerWithNotify::new(MempoolLimits::for_testing()));

        let nonce_info = NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE));
        let mempool_tx = MempoolTransaction {
            tx: tx_account_v0_valid.into_blockifier().unwrap().0,
            arrived_at: TxTimestamp::now(),
            converted_class: None,
            nonce: nonce_info.nonce,
            nonce_next: nonce_info.nonce_next,
        };
        mempool.insert_tx(mempool_tx.clone(), false, true, nonce_info).await.unwrap();

        let first_consumer = mempool.get_consumer_wait_for_ready_tx().await;
        // don't consume txs from first consumer

        // keep the first consumer around for now

        tracing::debug!("Hiii");
        let mut fut = Box::pin(mempool.get_consumer_wait_for_ready_tx());
        tracing::debug!("Hiii 2");

        // poll once
        assert!(fut.as_mut().now_or_never().is_none());
        tracing::debug!("Hiii 3");

        drop(first_consumer); // first consumer is dropped while there are still executable txs in the queue, this will wake up second consumer.
        tracing::debug!("Hiii 4");

        // poll once
        let mut consumer = fut.as_mut().now_or_never().unwrap();
        tracing::debug!("Hiii 5");
        let received = consumer.next().unwrap();
        tracing::debug!("Hiii 6");
        assert_eq!(received.contract_address(), mempool_tx.contract_address());
        assert_eq!(received.nonce(), mempool_tx.nonce());
        assert_eq!(received.tx_hash(), mempool_tx.tx_hash());
    }

    #[rstest::rstest]
    #[tokio::test]
    /// This is unused as of yet in madara, but the mempool supports having multiple waiters on the notify.
    /// Tests that the second consumer is not woken up if there are no more txs in the mempool.
    async fn test_mempool_notify_multiple_listeners_not_woken(tx_account_v0_valid: ValidatedMempoolTx) {
        let mempool = Arc::new(MempoolInnerWithNotify::new(MempoolLimits::for_testing()));

        let nonce_info = NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE));
        let mempool_tx = MempoolTransaction {
            tx: tx_account_v0_valid.into_blockifier().unwrap().0,
            arrived_at: TxTimestamp::now(),
            converted_class: None,
            nonce: nonce_info.nonce,
            nonce_next: nonce_info.nonce_next,
        };
        mempool.insert_tx(mempool_tx.clone(), false, true, nonce_info).await.unwrap();

        let mut first_consumer = mempool.get_consumer_wait_for_ready_tx().await;
        // consume!
        let received = first_consumer.next().unwrap();
        assert_eq!(received.contract_address(), mempool_tx.contract_address());
        assert_eq!(received.nonce(), mempool_tx.nonce());
        assert_eq!(received.tx_hash(), mempool_tx.tx_hash());

        // keep the first consumer around for now

        let mut fut = Box::pin(mempool.get_consumer_wait_for_ready_tx());

        // poll once
        assert!(fut.as_mut().now_or_never().is_none());

        drop(first_consumer); // first consumer is dropped while there are still executable txs in the queue, this will wake up second consumer.

        // poll once
        assert!(fut.as_mut().now_or_never().is_none()); // still waiting
    }
}
