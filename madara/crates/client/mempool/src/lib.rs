use anyhow::Context;
use dashmap::DashMap;
use mc_db::{rocksdb::RocksDBStorage, MadaraBackend, MadaraStorageRead, MadaraStorageWrite};
use metrics::MempoolMetrics;
use mp_transactions::validated::{TxTimestamp, ValidatedToBlockifierTxError, ValidatedTransaction};
use mp_utils::service::ServiceContext;
use notify::MempoolInnerWithNotify;
use starknet_api::core::Nonce;
use starknet_types_core::felt::Felt;
use std::sync::Arc;
use std::time::Duration;
use topic_pubsub::TopicWatchPubsub;
use transaction_status::{PreConfirmationStatus, TransactionStatus};

mod chain_watcher_task;
mod inner;
mod notify;
mod topic_pubsub;
mod transaction_status;

pub use inner::*;
pub use notify::MempoolWriteAccess;

pub mod metrics;

#[derive(thiserror::Error, Debug)]
pub enum MempoolInsertionError {
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
    #[error(transparent)]
    InnerMempool(#[from] TxInsertionError),
    #[error("Converting validated transaction: {0:#}")]
    ValidatedToBlockifier(#[from] ValidatedToBlockifierTxError),
    #[error("Invalid nonce")]
    InvalidNonce,
}

#[derive(Debug, Clone)]
pub struct MempoolConfig {
    pub save_to_db: bool,
}

impl Default for MempoolConfig {
    fn default() -> Self {
        Self { save_to_db: true }
    }
}

impl MempoolConfig {
    pub fn with_save_to_db(mut self, save_to_db: bool) -> Self {
        self.save_to_db = save_to_db;
        self
    }
}

/// Mempool also holds all of the transaction statuses.
pub struct Mempool<D: MadaraStorageRead = RocksDBStorage> {
    backend: Arc<MadaraBackend<D>>,
    inner: MempoolInnerWithNotify,
    metrics: MempoolMetrics,
    config: MempoolConfig,
    ttl: Option<Duration>,
    /// Pubsub for transaction statuses.
    watch_transaction_status: TopicWatchPubsub<Felt, Option<TransactionStatus>>,
    /// All current transaction statuses for mempool & preconfirmed block.
    preconfirmed_transactions_statuses: DashMap<Felt, PreConfirmationStatus>,
}

impl<D: MadaraStorageRead> Mempool<D> {
    pub fn new(backend: Arc<MadaraBackend<D>>, config: MempoolConfig) -> Self {
        Mempool {
            inner: MempoolInnerWithNotify::new(backend.chain_config()),
            ttl: backend.chain_config().mempool_ttl,
            backend,
            config,
            metrics: MempoolMetrics::register(),
            watch_transaction_status: Default::default(),
            preconfirmed_transactions_statuses: Default::default(),
        }
    }
}

impl<D: MadaraStorageRead + MadaraStorageWrite> Mempool<D> {
    pub async fn load_txs_from_db(&self) -> Result<(), anyhow::Error> {
        if !self.config.save_to_db {
            // If saving is disabled, we don't want to read from db. Otherwise, if there are txs in the database, they will be re-inserted
            // everytime we restart the node, but will never be removed from db once they're consumed.
            return Ok(());
        }
        for res in self.backend.get_saved_mempool_transactions() {
            let tx = res.context("Getting mempool transactions")?;
            let is_new_tx = false; // do not trigger metrics update and db update.
            if let Err(err) = self.add_tx(tx, is_new_tx).await {
                match err {
                    MempoolInsertionError::InnerMempool(TxInsertionError::TooOld { .. }) => {} // do nothing
                    err => tracing::warn!("Could not re-add mempool transaction from db: {err:#}"),
                }
            }
        }
        Ok(())
    }

    /// Accept a new validated transaction.
    pub async fn accept_tx(&self, tx: ValidatedTransaction) -> Result<(), MempoolInsertionError> {
        self.add_tx(tx, /* is_new_tx */ true).await
    }

    /// Use `is_new_tx: false` when loading transactions from db, so that we skip saving in db and updating metrics.
    async fn add_tx(&self, tx: ValidatedTransaction, is_new_tx: bool) -> Result<(), MempoolInsertionError> {
        tracing::debug!("Accepting transaction tx_hash={:#x} is_new_tx={is_new_tx}", tx.hash);

        let now = TxTimestamp::now();
        let account_nonce =
            self.backend.view_on_latest().get_contract_nonce(&tx.contract_address)?.unwrap_or(Felt::ZERO);
        let mut removed_txs = smallvec::SmallVec::<[ValidatedTransaction; 1]>::new();
        // Guard is immediately dropped.
        let ret = self.inner.write().await.insert_tx(now, tx.clone(), Nonce(account_nonce), &mut removed_txs);
        self.on_txs_removed(&removed_txs);
        if ret.is_ok() {
            self.on_tx_added(&tx, is_new_tx);
        }
        ret.map_err(Into::into)
    }

    /// Update secondary state when a new transaction has been successfully added to the mempool.
    /// Use `is_new_tx: false` when loading transactions from db, so that we skip saving in db and updating metrics.
    fn on_tx_added(&self, tx: &ValidatedTransaction, is_new_tx: bool) {
        tracing::debug!("Accepted transaction tx_hash={:#x}", tx.hash);
        if is_new_tx {
            self.metrics.accepted_transaction_counter.add(1, &[]);
            if self.config.save_to_db {
                if let Err(err) = self.backend.write_saved_mempool_transaction(tx) {
                    tracing::error!("Could not add mempool transaction to database: {err:#}");
                }
            }
        }

        if let dashmap::Entry::Vacant(entry) = self.preconfirmed_transactions_statuses.entry(tx.hash) {
            let status = PreConfirmationStatus::Received(Arc::new(tx.clone()));
            entry.insert(status.clone());
            self.watch_transaction_status.publish(&tx.hash, Some(TransactionStatus::Preconfirmed(status)));
        }
    }

    /// Update secondary state when a new transaction has been successfully removed from the mempool.
    fn on_txs_removed(&self, removed: &[ValidatedTransaction]) {
        if self.config.save_to_db {
            if let Err(err) = self.backend.remove_saved_mempool_transactions(removed.iter().map(|tx| tx.hash)) {
                tracing::error!("Could not remove mempool transactions from database: {err:#}");
            }
        }

        for tx in removed {
            if let dashmap::Entry::Occupied(entry) = self.preconfirmed_transactions_statuses.entry(tx.hash) {
                if matches!(entry.get(), PreConfirmationStatus::Received(_)) {
                    entry.remove();
                    self.watch_transaction_status.publish(&tx.hash, None);
                }
            }
        }
    }

    async fn remove_ttl_exceeded_txs(&self) -> anyhow::Result<()> {
        let mut removed_txs = smallvec::SmallVec::<[ValidatedTransaction; 1]>::new();
        let now = TxTimestamp::now();
        {
            let mut lock = self.inner.write().await;
            lock.remove_all_ttl_exceeded_txs(now, &mut removed_txs);
        }
        self.on_txs_removed(&removed_txs);
        Ok(())
    }

    pub async fn run_mempool_task(&self, ctx: ServiceContext) -> anyhow::Result<()> {
        self.load_txs_from_db().await.context("Loading transactions from db on mempool startup.")?;

        tokio::try_join!(self.run_ttl_task(ctx.clone()), self.run_chain_watcher_task(ctx))?;
        Ok(())
    }

    pub async fn run_ttl_task(&self, mut ctx: ServiceContext) -> anyhow::Result<()> {
        if self.ttl.is_none() {
            // no need to do anything more
            ctx.cancelled().await;
            return Ok(());
        }

        let mut interval = tokio::time::interval(Duration::from_secs(5));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = ctx.cancelled() => return Ok(()),
                _ = interval.tick() => self.remove_ttl_exceeded_txs().await.context("Removing TTL-exceeded txs.")?,
            }
        }
    }

    pub async fn is_empty(&self) -> bool {
        self.inner.read().await.is_empty()
    }

    pub async fn get_transaction<R>(
        &self,
        contract_address: Felt,
        nonce: Felt,
        f: impl FnOnce(&ValidatedTransaction) -> R,
    ) -> Option<R> {
        let lock = self.inner.read().await;
        lock.get_transaction(&contract_address.try_into().ok()?, &Nonce(nonce)).map(f)
    }

    #[tracing::instrument(skip(self), fields(module = "Mempool"))]
    /// Returns a view of the mempool intended for consuming transactions from the mempool.
    /// If the mempool has no mempool that can be consumed, this function will wait until there is at least 1 transaction to consume.
    /// This holds the lock to the inner mempool - use with care.
    pub async fn get_consumer(&self) -> MempoolConsumer {
        MempoolConsumer { lock: self.inner.get_write_access_wait_for_ready().await }
    }
}

/// A view into the mempool, intended for consuming transactions. This is expected to be used by block production to
/// pop transactions from the mempool and execute them.
///
/// This struct implements [`Iterator`] by popping the next transaction to execute from the mempool.
///
/// This holds the lock to the inner mempool - use with care.
pub struct MempoolConsumer {
    lock: MempoolWriteAccess,
}
impl Iterator for MempoolConsumer {
    type Item = ValidatedTransaction;
    fn next(&mut self) -> Option<Self::Item> {
        self.lock.pop_next_ready()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let n_ready = self.lock.ready_transactions();
        (n_ready, Some(n_ready))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use starknet_api::{core::ContractAddress, transaction::TransactionHash};
    use std::time::Duration;

    #[rstest::fixture]
    async fn backend() -> Arc<mc_db::MadaraBackend> {
        let backend = mc_db::MadaraBackend::open_for_testing(Arc::new(mp_chain_config::ChainConfig::madara_test()));
        let mut genesis = mc_devnet::ChainGenesisDescription::base_config().unwrap();
        genesis.add_devnet_contracts(10).unwrap();
        genesis.build_and_store(&backend).await.unwrap();
        backend
    }

    const CONTRACT_ADDRESS: Felt =
        Felt::from_hex_unchecked("0x055be462e718c4166d656d11f89e341115b8bc82389c3762a10eade04fcb225d");

    #[rstest::fixture]
    pub fn tx_account(#[default(CONTRACT_ADDRESS)] contract_address: Felt) -> ValidatedTransaction {
        use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
        static HASH: AtomicU64 = AtomicU64::new(0);
        let tx_hash = TransactionHash(HASH.fetch_add(1, Relaxed).into());

        ValidatedTransaction::from_starknet_api(
            starknet_api::executable_transaction::AccountTransaction::Invoke(
                starknet_api::executable_transaction::InvokeTransaction {
                    tx: starknet_api::transaction::InvokeTransaction::V3(
                        starknet_api::transaction::InvokeTransactionV3 {
                            sender_address: ContractAddress::try_from(contract_address).unwrap(),
                            resource_bounds: Default::default(),
                            tip: Default::default(),
                            signature: Default::default(),
                            nonce: Default::default(),
                            calldata: Default::default(),
                            nonce_data_availability_mode: Default::default(),
                            fee_data_availability_mode: Default::default(),
                            paymaster_data: Default::default(),
                            account_deployment_data: Default::default(),
                        },
                    ),
                    tx_hash,
                },
            ),
            TxTimestamp::now(),
            None,
        )
    }

    #[rstest::rstest]
    #[timeout(Duration::from_millis(1_000))]
    #[tokio::test]
    async fn mempool_accept_tx_pass(#[future] backend: Arc<mc_db::MadaraBackend>, tx_account: ValidatedTransaction) {
        let backend = backend.await;
        let mempool = Mempool::new(backend, MempoolConfig::default());
        let result = mempool.accept_tx(tx_account).await;
        assert_matches::assert_matches!(result, Ok(()));

        mempool.inner.read().await.check_invariants();
    }

    /// This test makes sure that taking a transaction from the mempool works as
    /// intended.
    #[rstest::rstest]
    #[timeout(Duration::from_millis(1_000))]
    #[tokio::test]
    async fn mempool_take_tx_pass(#[future] backend: Arc<mc_db::MadaraBackend>, mut tx_account: ValidatedTransaction) {
        let backend = backend.await;
        let mempool = Mempool::new(backend, MempoolConfig::default());
        let timestamp = TxTimestamp::now();
        tx_account.arrived_at = timestamp;
        let result = mempool.accept_tx(tx_account).await;
        assert_matches::assert_matches!(result, Ok(()));

        let mempool_tx = mempool.get_consumer().await.next().expect("Mempool should contain a transaction");
        assert_eq!(mempool_tx.arrived_at, timestamp);

        assert!(mempool.is_empty().await, "Mempool should be empty");

        mempool.inner.read().await.check_invariants();
    }
}
