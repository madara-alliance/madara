use crate::{InnerMempool, MempoolConfig};
use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
};
use tokio::sync::{Notify, OwnedRwLockWriteGuard, RwLock, RwLockReadGuard};

/// Write access to the mempool. Holds a lock.
/// When dropped, if there are ready transactions in the mempool, this will notify waiters.
pub struct MempoolWriteAccess {
    notify: Arc<Notify>,
    inner: OwnedRwLockWriteGuard<InnerMempool>,
}
impl Deref for MempoolWriteAccess {
    type Target = InnerMempool;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl DerefMut for MempoolWriteAccess {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl Drop for MempoolWriteAccess {
    fn drop(&mut self) {
        // If there are still ready transactions in the mempool, notify the next waiter.
        if self.inner.has_ready_transactions() {
            tracing::debug!("notify_one (drop)");
            self.notify.notify_one();
        }
    }
}

pub(crate) struct MempoolInnerWithNotify {
    inner: Arc<RwLock<InnerMempool>>,
    // Notify listener when the mempool goes from !has_ready_transactions to has_ready_transactions.
    notify: Arc<Notify>,
}
impl MempoolInnerWithNotify {
    pub fn new(config: MempoolConfig) -> Self {
        Self {
            inner: RwLock::new(InnerMempool::new(crate::InnerMempoolConfig {
                score_function: config.score_function,
                max_transactions: config.max_transactions,
                max_declare_transactions: config.max_declare_transactions,
                ttl: config.ttl,
            }))
            .into(),
            notify: Default::default(),
        }
    }

    /// Returns a reading view of the inner mempool.
    pub async fn read(&self) -> RwLockReadGuard<'_, InnerMempool> {
        self.inner.read().await
    }

    /// Returns a view of the mempool intended for consuming transactions from the mempool.
    /// If the mempool has no transaction that can be consumed, this function will wait until there is at least 1 transaction to consume.
    pub async fn get_write_access_wait_for_ready(&self) -> MempoolWriteAccess {
        let permit = self.notify.notified(); // This doesn't actually register us to be notified yet.
        tokio::pin!(permit);
        loop {
            {
                tracing::debug!("taking lock");
                let inner = self.inner.clone().write_owned().await;

                if inner.has_ready_transactions() {
                    tracing::debug!("consumer ready");
                    return MempoolWriteAccess { inner, notify: self.notify.clone() };
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
    pub async fn write(&self) -> MempoolWriteAccess {
        MempoolWriteAccess { notify: self.notify.clone(), inner: self.inner.clone().write_owned().await }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::tx_account;
    use futures::FutureExt;
    use mp_convert::Felt;
    use mp_transactions::validated::ValidatedMempoolTx;
    use starknet_api::core::Nonce;
    use std::sync::Arc;

    #[rstest::rstest]
    #[tokio::test]
    async fn test_mempool_notify(tx_account: ValidatedMempoolTx) {
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
            .insert_tx(tx_account.arrived_at, tx_account.clone(), Nonce(Felt::ZERO), &mut vec![])
            .unwrap();

        // poll once
        let mut consumer = fut.as_mut().now_or_never().unwrap();
        let received = consumer.pop_next_ready().unwrap();
        assert_eq!(received.contract_address, tx_account.contract_address);
        assert_eq!(received.tx.nonce(), tx_account.tx.nonce());
        assert_eq!(received.tx_hash, tx_account.tx_hash);
    }

    #[rstest::rstest]
    #[tokio::test]
    /// This is unused as of yet in madara, but the mempool supports having multiple waiters on the notify.
    async fn test_mempool_notify_multiple_listeners(tx_account: ValidatedMempoolTx) {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer()
            .try_init();
        let mempool = Arc::new(MempoolInnerWithNotify::new(MempoolConfig::for_testing()));

        mempool
            .write()
            .await
            .insert_tx(tx_account.arrived_at, tx_account.clone(), Nonce(Felt::ZERO), &mut vec![])
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
        assert_eq!(received.contract_address, tx_account.contract_address);
        assert_eq!(received.tx.nonce(), tx_account.tx.nonce());
        assert_eq!(received.tx_hash, tx_account.tx_hash);
    }

    #[rstest::rstest]
    #[tokio::test]
    /// This is unused as of yet in madara, but the mempool supports having multiple waiters on the notify.
    /// Tests that the second consumer is not woken up if there are no more txs in the mempool.
    async fn test_mempool_notify_multiple_listeners_not_woken(tx_account: ValidatedMempoolTx) {
        let mempool = Arc::new(MempoolInnerWithNotify::new(MempoolConfig::for_testing()));

        mempool
            .write()
            .await
            .insert_tx(tx_account.arrived_at, tx_account.clone(), Nonce(Felt::ZERO), &mut vec![])
            .unwrap();

        let mut first_consumer = mempool.get_write_access_wait_for_ready().await;
        // consume!
        let received = first_consumer.pop_next_ready().unwrap();
        assert_eq!(received.contract_address, tx_account.contract_address);
        assert_eq!(received.tx.nonce(), tx_account.tx.nonce());
        assert_eq!(received.tx_hash, tx_account.tx_hash);

        // keep the first consumer around for now

        let mut fut = Box::pin(mempool.get_write_access_wait_for_ready());

        // poll once
        assert!(fut.as_mut().now_or_never().is_none());

        drop(first_consumer); // first consumer is dropped while there are still executable txs in the queue, this will wake up second consumer.

        // poll once
        assert!(fut.as_mut().now_or_never().is_none()); // still waiting
    }
}
