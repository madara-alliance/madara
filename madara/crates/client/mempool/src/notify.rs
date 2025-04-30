use crate::{MempoolInner, MempoolLimits, MempoolTransaction, TxInsertionError};
use mc_db::mempool_db::NonceInfo;
use mp_convert::{Felt, ToFelt};
use starknet_api::core::Nonce;
use std::{
    collections::BTreeMap,
    sync::{RwLock, RwLockReadGuard, RwLockWriteGuard},
};
use tokio::sync::Notify;

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

// todo(deadlocks): document mutex ordering
pub(crate) struct MempoolInnerWithNotify {
    // TODO(perf): figure out if it's worth to have a tokio lock here instead of std lock.
    // (ie. use tracing spans to log how long the write lock is taken)
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
    pub fn insert_tx(
        &self,
        mempool_tx: MempoolTransaction,
        force: bool,
        update_limits: bool,
        nonce_info: NonceInfo,
    ) -> Result<(), TxInsertionError> {
        let mut lock = self.inner.write().expect("Poisoned lock");
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
    pub fn read(&self) -> RwLockReadGuard<'_, MempoolInner> {
        self.inner.read().expect("Poisoned lock")
    }

    pub fn nonce_cache_read(&self) -> RwLockReadGuard<'_, BTreeMap<Felt, Nonce>> {
        self.nonce_cache.read().expect("Poisoned lock")
    }

    #[cfg(test)]
    pub fn write(&self) -> RwLockWriteGuard<'_, MempoolInner> {
        self.inner.write().expect("Poisoned lock")
    }
    #[cfg(test)]
    pub fn nonce_cache_write(&self) -> RwLockWriteGuard<'_, BTreeMap<Felt, Nonce>> {
        self.nonce_cache.write().expect("Poisoned lock")
    }

    /// Returns a view of the mempool intended for consuming transactions from the mempool.
    /// If the mempool has no transaction that can be consumed, this function will wait until there is at least 1 transaction to consume.
    pub async fn get_consumer_wait_for_ready_tx(&self) -> MempoolConsumerView<'_> {
        let permit = self.notify.notified(); // This doesn't actually register us to be notified yet.
        tokio::pin!(permit);
        loop {
            {
                tracing::debug!("taking lock");
                let nonce_cache = self.nonce_cache.write().expect("Poisoned lock");
                let inner = self.inner.write().expect("Poisoned lock");

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
    pub fn get_consumer(&self) -> MempoolConsumerView<'_> {
        MempoolConsumerView {
            notify: &self.notify,
            nonce_cache: self.nonce_cache.write().expect("Poisoned lock"),
            inner: self.inner.write().expect("Poisoned lock"),
        }
    }
}
