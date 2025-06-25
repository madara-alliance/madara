use crate::{InnerMempool, MempoolConfig};
use std::ops::{Deref, DerefMut};
use tokio::sync::{Notify, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Write access to the mempool. Holds a lock.
/// When dropped, if there are ready transactions in the mempool, this will notify waiters.
pub struct MempoolWriteAccess<'a> {
    notify: &'a Notify,
    inner: RwLockWriteGuard<'a, InnerMempool>,
}
impl<'a> Deref for MempoolWriteAccess<'a> {
    type Target = InnerMempool;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl<'a> DerefMut for MempoolWriteAccess<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl Drop for MempoolWriteAccess<'_> {
    fn drop(&mut self) {
        // If there are still ready transactions in the mempool, notify the next waiter.
        if self.inner.has_ready_transactions() {
            tracing::debug!("notify_one (drop)");
            self.notify.notify_one();
        }
    }
}

pub(crate) struct MempoolInnerWithNotify {
    inner: RwLock<InnerMempool>,
    // Notify listener when the mempool goes from !has_ready_transactions to has_ready_transactions.
    notify: Notify,
}
impl MempoolInnerWithNotify {
    pub fn new(config: MempoolConfig) -> Self {
        Self {
            inner: RwLock::new(InnerMempool::new(crate::InnerMempoolConfig {
                score_function: config.score_function,
                max_transactions: config.max_transactions,
                max_declare_transactions: config.max_declare_transactions,
                ttl: config.ttl,
            })),
            notify: Default::default(),
        }
    }

    /// Returns a reading view of the inner mempool.
    pub async fn read(&self) -> RwLockReadGuard<'_, InnerMempool> {
        self.inner.read().await
    }

    /// Returns a view of the mempool intended for consuming transactions from the mempool.
    /// If the mempool has no transaction that can be consumed, this function will wait until there is at least 1 transaction to consume.
    pub async fn get_write_access_wait_for_ready(&self) -> MempoolWriteAccess<'_> {
        let permit = self.notify.notified(); // This doesn't actually register us to be notified yet.
        tokio::pin!(permit);
        loop {
            {
                tracing::debug!("taking lock");
                let inner = self.inner.write().await;

                if inner.has_ready_transactions() {
                    tracing::debug!("consumer ready");
                    return MempoolWriteAccess { inner, notify: &self.notify };
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
    pub async fn write(&self) -> MempoolWriteAccess<'_> {
        MempoolWriteAccess { notify: &self.notify, inner: self.inner.write().await }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::tx_account_v0_valid;
    use futures::FutureExt;
    use mp_convert::Felt;
    use mp_transactions::validated::ValidatedMempoolTx;
    use starknet_api::core::Nonce;
    use std::sync::Arc;

    #[rstest::rstest]
    #[tokio::test]
    async fn test_mempool_notify(tx_account_v0_valid: ValidatedMempoolTx) {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer()
            .try_init();
        let mempool = Arc::new(MempoolInnerWithNotify::new(MempoolConfig::for_testing()));

        let mut fut = Box::pin(mempool.get_write_access_wait_for_ready());

        // poll once
        assert!(fut.as_mut().now_or_never().is_none());

        mempool
            .write()
            .await
            .insert_tx(tx_account_v0_valid.arrived_at, tx_account_v0_valid.clone(), Nonce(Felt::ZERO), &mut vec![])
            .unwrap();

        // poll once
        let mut consumer = fut.as_mut().now_or_never().unwrap();
        let received = consumer.pop_next_ready().unwrap();
        assert_eq!(received.contract_address, tx_account_v0_valid.contract_address);
        assert_eq!(received.tx.nonce(), tx_account_v0_valid.tx.nonce());
        assert_eq!(received.tx_hash, tx_account_v0_valid.tx_hash);
    }

    #[rstest::rstest]
    #[tokio::test]
    /// This is unused as of yet in madara, but the mempool supports having multiple waiters on the notify.
    async fn test_mempool_notify_multiple_listeners(tx_account_v0_valid: ValidatedMempoolTx) {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer()
            .try_init();
        let mempool = Arc::new(MempoolInnerWithNotify::new(MempoolConfig::for_testing()));

        mempool
            .write()
            .await
            .insert_tx(tx_account_v0_valid.arrived_at, tx_account_v0_valid.clone(), Nonce(Felt::ZERO), &mut vec![])
            .unwrap();

        let first_consumer = mempool.get_write_access_wait_for_ready().await;
        // don't consume txs from first consumer

        // keep the first consumer around during this call
        let mut fut = Box::pin(mempool.get_write_access_wait_for_ready());

        // poll once
        assert!(fut.as_mut().now_or_never().is_none());

        drop(first_consumer); // first consumer is dropped while there are still executable txs in the queue, this will wake up second consumer.

        // poll once
        let mut consumer = fut.as_mut().now_or_never().unwrap();
        let received = consumer.pop_next_ready().unwrap();
        assert_eq!(received.contract_address, tx_account_v0_valid.contract_address);
        assert_eq!(received.tx.nonce(), tx_account_v0_valid.tx.nonce());
        assert_eq!(received.tx_hash, tx_account_v0_valid.tx_hash);
    }

    #[rstest::rstest]
    #[tokio::test]
    /// This is unused as of yet in madara, but the mempool supports having multiple waiters on the notify.
    /// Tests that the second consumer is not woken up if there are no more txs in the mempool.
    async fn test_mempool_notify_multiple_listeners_not_woken(tx_account_v0_valid: ValidatedMempoolTx) {
        let mempool = Arc::new(MempoolInnerWithNotify::new(MempoolConfig::for_testing()));

        mempool
            .write()
            .await
            .insert_tx(tx_account_v0_valid.arrived_at, tx_account_v0_valid.clone(), Nonce(Felt::ZERO), &mut vec![])
            .unwrap();

        let mut first_consumer = mempool.get_write_access_wait_for_ready().await;
        // consume!
        let received = first_consumer.pop_next_ready().unwrap();
        assert_eq!(received.contract_address, tx_account_v0_valid.contract_address);
        assert_eq!(received.tx.nonce(), tx_account_v0_valid.tx.nonce());
        assert_eq!(received.tx_hash, tx_account_v0_valid.tx_hash);

        // keep the first consumer around for now

        let mut fut = Box::pin(mempool.get_write_access_wait_for_ready());

        // poll once
        assert!(fut.as_mut().now_or_never().is_none());

        drop(first_consumer); // first consumer is dropped while there are still executable txs in the queue, this will wake up second consumer.

        // poll once
        assert!(fut.as_mut().now_or_never().is_none()); // still waiting
    }
}
