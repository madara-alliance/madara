//! Madara mempool. This crate manages the transaction pool for sequencer nodes, accepting
//! transactions from RPC endpoints and organizing them for block production.
//!
//! # Overview
//!
//! The Mempool stores transactions waiting to be included in blocks. It efficiently indexes
//! transactions for insertion, retrieval and deletion using several coordinated queues that
//! maintain different orderings. Each queue serves a specific purpose: finding ready transactions,
//! expiring old ones, looking up by hash, or choosing which to evict when full.
//!
//! The mempool is split into two layers: the outer [`Mempool`] handles persistence and metrics,
//! while the inner [`InnerMempool`] manages the actual transaction storage and ordering logic.
//!
//! # Transaction Ordering
//!
//! The inner mempool maintains four specialized queues that work together:
//!
//! - [`ready_queue`]: Orders accounts that have transactions ready to execute (where the
//!   transaction nonce matches the account nonce)
//! - [`timestamp_queue`]: Orders all transactions by arrival time for TTL expiration
//! - [`by_tx_hash`]: Maps transaction hashes to their location for quick lookups
//! - [`eviction_queue`]: Orders accounts by eviction priority when the mempool is full
//!
//! # Dynamic Scoring
//!
//! Transaction priority is determined by a [`ScoreFunction`] enum which supports two modes:
//!
//! - **FCFS mode**: First-come-first-served based on timestamp
//! - **Tip mode**: Prioritizes by tip amount with minimum bump requirements
//!
//! While the scoring function is an enum that could theoretically be swapped at runtime, this
//! functionality is not yet implemented. The mode is set at startup based on chain configuration.
//!
//! # Transaction Insertion Flow
//!
//! When a transaction is inserted into the mempool:
//!
//! 1. **It is received by the outer mempool** via [`add_tx`], which handles metrics and
//!    forwards the transaction to the inner mempool.
//! 2. **The transaction is forwarded to the inner mempool** via [`insert_tx`]. This handles the
//!    actual insertion and eviction logic.
//! 3. **Pre-insertion checks are performed**:
//!    - Declare transactions are rejected if they do not follow the current account nonce.
//!    - For a transaction to be replaced it needs to offer a sufficient tip bump.
//!    - Duplicate transactions are rejected
//!    - TTL is verified
//! 4. **Space check**: If the mempool is full, we attempt eviction (see next section)
//! 5. **Insertion**: The transaction is added to the primary accounts structure
//! 6. **Update propagation**: All queues are updated to reflect the change
//!
//! # Transaction Eviction
//!
//! When the mempool reaches capacity:
//!
//! 1. **We look for an eviction candidate**: we check the `eviction_queue` for the account with the
//!    highest eviction score (accounts with transactions furthest from being executable).
//! 2. **If the new transaction has lower priority** than the worst transaction in the mempool, it
//!    is rejected.
//! 3. **Make room**: remove the last transaction from account chosen in step 1.
//! 4. **Insertion**: The transaction is added to the primary accounts structure
//! 5. **Update propagation**: All queues are updated to reflect the change
//!
//! The eviction score considers both nonce distance (how far from executable) and transaction
//! score within that distance tier.
//!
//! # Mempool Updates
//!
//! After any change (insertion, removal, or nonce update), the mempool maintains consistency
//! through a modular update system. This is backed by the [`AccountUpdate`] struct, which describes
//! what changes took place (which account was affect, which transactions were added/removed).
//!
//! Based on this, the [`AccountUpdate`] is the applied to each inner mempool queue in the following
//! order:
//!
//!    1. `ready_queue` (update ready status)
//!    2. `timestamp_queue` (add/remove by timestamp)
//!    3. `by_tx_hash` (update hash lookups)
//!    4. `eviction_queue` (update eviction priorities)
//!    5. `limiter` (track transaction counts)
//!
//! Finally, we return a list of all transactions which have been removed in the process. This
//! design keeps the invariants modular and makes them easier to update in the future.
//!
//! # Reading from the Mempool
//!
//! The mempool provides several ways to read transactions:
//!
//! ## Block Production via MempoolConsumer
//!
//! The [`MempoolConsumer`] provides an iterator interface for block production. It acquires a
//! write lock on the inner mempool and pops transactions in priority order. **Warning**: This
//! holds the lock, preventing new transactions from being added. Use sparingly to avoid deadlocks.
//!
//! ```no_run
//! # async fn example(mempool: &mc_mempool::Mempool) {
//! let consumer = mempool.get_consumer().await;
//! for tx in consumer {
//!     // Process transaction
//! }
//! # }
//! ```
//!
//! ## Notifications via tx_sender
//!
//! The `tx_sender` broadcast channel sends a continuous stream of transaction hashes as they are
//! added to the mempool. This is used by `mc-rpc` to implement transaction status subscriptions.
//!
//! ## Quick Checks via received_txs
//!
//! The `received_txs` set allows checking if a transaction exists without locking the inner
//! mempool. This is useful for quick status checks from RPC methods.
//!
//! [`ready_queue`]: InnerMempool::ready_queue
//! [`timestamp_queue`]: InnerMempool::timestamp_queue
//! [`by_tx_hash`]: InnerMempool::by_tx_hash
//! [`eviction_queue`]: InnerMempool::eviction_queue
//! [`ScoreFunction`]: tx::ScoreFunction
//! [`add_tx`]: Mempool::add_tx
//! [`insert_tx`]: InnerMempool::insert_tx
//! [`AccountUpdate`]: accounts::AccountUpdate
//! [`MempoolConsumer`]: crate::MempoolConsumer

use anyhow::Context;
use dashmap::DashMap;
use mc_db::{rocksdb::RocksDBStorage, MadaraBackend, MadaraStorageRead, MadaraStorageWrite};
use metrics::{ExternalDbOutboxMetrics, MempoolMetrics};
use mp_convert::ToFelt;
use mp_transactions::validated::{TxTimestamp, ValidatedToBlockifierTxError, ValidatedTransaction};
use mp_utils::service::ServiceContext;
use notify::MempoolInnerWithNotify;
use starknet_api::core::Nonce;
use starknet_types_core::felt::Felt;
use std::time::Duration;
use std::{collections::HashMap, sync::Arc};
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
    pub external_outbox: ExternalOutboxConfig,
}

impl Default for MempoolConfig {
    fn default() -> Self {
        Self { save_to_db: true, external_outbox: ExternalOutboxConfig::default() }
    }
}

impl MempoolConfig {
    pub fn with_save_to_db(mut self, save_to_db: bool) -> Self {
        self.save_to_db = save_to_db;
        self
    }

    pub fn with_external_outbox(mut self, external_outbox: ExternalOutboxConfig) -> Self {
        self.external_outbox = external_outbox;
        self
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ExternalOutboxConfig {
    pub enabled: bool,
    pub strict: bool,
}

impl ExternalOutboxConfig {
    pub fn enabled(strict: bool) -> Self {
        Self { enabled: true, strict }
    }
}

impl Default for ExternalOutboxConfig {
    fn default() -> Self {
        Self { enabled: false, strict: true }
    }
}

/// Mempool also holds all of the transaction statuses.
pub struct Mempool<D: MadaraStorageRead = RocksDBStorage> {
    backend: Arc<MadaraBackend<D>>,
    inner: MempoolInnerWithNotify,
    metrics: MempoolMetrics,
    external_db_outbox_metrics: ExternalDbOutboxMetrics,
    config: MempoolConfig,
    external_outbox: ExternalOutboxConfig,
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
            external_outbox: config.external_outbox,
            config,
            metrics: MempoolMetrics::register(),
            external_db_outbox_metrics: ExternalDbOutboxMetrics::register(),
            watch_transaction_status: Default::default(),
            preconfirmed_transactions_statuses: Default::default(),
        }
    }
}

impl<D: MadaraStorageRead + MadaraStorageWrite> Mempool<D> {
    async fn load_txs_from_db(&self) -> Result<(), anyhow::Error> {
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
        self.reconcile_loaded_txs_with_chain_head().await?;
        Ok(())
    }

    async fn reconcile_loaded_txs_with_chain_head(&self) -> Result<(), anyhow::Error> {
        let contract_addresses = {
            let guard = self.inner.read().await;
            guard.contract_addresses().map(|address| address.to_felt()).collect::<Vec<_>>()
        };
        if contract_addresses.is_empty() {
            return Ok(());
        }

        let head = self.backend.chain_head_state();
        let confirmed_view = self.backend.view_on_latest_confirmed();
        let mut nonce_updates = HashMap::with_capacity(contract_addresses.len());

        let mut preconfirmed_nonce_overrides = HashMap::new();
        if let Some(internal_preconfirmed_tip) = head.internal_preconfirmed_tip {
            let start_block_n = head.confirmed_tip.map(|n| n.saturating_add(1)).unwrap_or(0);
            for block_number in start_block_n..=internal_preconfirmed_tip {
                let preconfirmed_view = self
                    .backend
                    .block_view_on_preconfirmed(block_number)
                    .with_context(|| format!("Missing preconfirmed block #{block_number} during mempool startup"))?;
                for executed_tx in preconfirmed_view.borrow_content().executed_transactions() {
                    preconfirmed_nonce_overrides
                        .extend(executed_tx.state_diff.nonces.iter().map(|(&addr, &nonce)| (addr, nonce)));
                }
            }
        }

        for contract_address in contract_addresses {
            let account_nonce = preconfirmed_nonce_overrides
                .get(&contract_address)
                .copied()
                .or(confirmed_view.get_contract_nonce(&contract_address)?)
                .unwrap_or(Felt::ZERO);
            nonce_updates.insert(contract_address, account_nonce);
        }

        self.update_account_nonces(nonce_updates).await
    }

    async fn update_account_nonces(&self, nonce_updates: HashMap<Felt, Felt>) -> Result<(), anyhow::Error> {
        if nonce_updates.is_empty() {
            return Ok(());
        }

        let mut removed_txs = smallvec::SmallVec::<[ValidatedTransaction; 1]>::new();
        let summary = {
            let mut guard = self.inner.write().await;
            for (contract_address, account_nonce) in nonce_updates {
                guard.update_account_nonce(
                    &contract_address.try_into().context("Invalid contract address")?,
                    &Nonce(account_nonce),
                    &mut removed_txs,
                );
            }
            guard.summary()
        };
        self.metrics.record_mempool_state(&summary);
        self.on_txs_removed(&removed_txs);

        Ok(())
    }

    /// Accept a new validated transaction.
    pub async fn accept_tx(&self, tx: ValidatedTransaction) -> Result<(), MempoolInsertionError> {
        self.add_tx(tx, /* is_new_tx */ true).await
    }

    /// Use `is_new_tx: false` when loading transactions from db, so that we skip saving in db and updating metrics.
    async fn add_tx(&self, tx: ValidatedTransaction, is_new_tx: bool) -> Result<(), MempoolInsertionError> {
        tracing::debug!("Accepting transaction tx_hash={:#x} is_new_tx={is_new_tx}", tx.hash);

        let mut outbox_id = None;
        if is_new_tx && self.external_outbox.enabled {
            match self.backend.write_external_outbox(&tx) {
                Ok(id) => {
                    outbox_id = Some(id);
                    self.external_db_outbox_metrics.outbox_writes.add(1, &[]);
                }
                Err(err) => {
                    tracing::error!("Could not write external outbox transaction: {err:#}");
                    self.external_db_outbox_metrics.outbox_write_errors.add(1, &[]);
                    if self.external_outbox.strict {
                        self.external_db_outbox_metrics.outbox_strict_rejections.add(1, &[]);
                        return Err(MempoolInsertionError::Internal(anyhow::anyhow!("outbox write failed: {err:#}")));
                    }
                }
            }
        }

        let now = TxTimestamp::now();
        let account_nonce =
            self.backend.view_on_latest().get_contract_nonce(&tx.contract_address)?.unwrap_or(Felt::ZERO);
        let mut removed_txs = smallvec::SmallVec::<[ValidatedTransaction; 1]>::new();

        let (ret, summary) = {
            let mut lock = self.inner.write().await;
            let ret = lock.insert_tx(now, tx.clone(), Nonce(account_nonce), &mut removed_txs);
            (ret, lock.summary())
        };

        if ret.is_err() {
            if let Some(outbox_id) = outbox_id {
                if let Err(err) = self.backend.delete_external_outbox(outbox_id) {
                    tracing::warn!("Failed to roll back external outbox write: {err:#}");
                    self.external_db_outbox_metrics.outbox_rollback_delete_errors.add(1, &[]);
                }
            }
        }

        self.metrics.record_mempool_state(&summary);

        self.on_txs_removed(&removed_txs);
        if ret.is_ok() {
            if removed_txs.is_empty() {
                tracing::info!("🔖 Inserted 1 transaction to the mempool [{summary}]");
            } else if removed_txs.len() == 1 {
                tracing::info!("🔖 Replaced 1 transaction in the mempool [{summary}]");
            } else {
                tracing::info!(
                    "🔖 Inserted 1 and removed {} transactions from the mempool [{summary}]",
                    removed_txs.len()
                );
            }
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
        if removed.is_empty() {
            return;
        }
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
        let summary = {
            let mut lock = self.inner.write().await;
            lock.remove_all_ttl_exceeded_txs(now, &mut removed_txs);
            lock.summary()
        };

        self.metrics.record_mempool_state(&summary);

        self.on_txs_removed(&removed_txs);
        if !removed_txs.is_empty() {
            tracing::info!(
                "🔖 Removed {} transactions from the mempool due to TTL limit [{summary}]",
                removed_txs.len()
            );
        }

        Ok(())
    }

    pub async fn run_mempool_task(&self, ctx: ServiceContext) -> anyhow::Result<()> {
        self.load_txs_from_db().await.context("Loading transactions from db on mempool startup.")?;

        tokio::try_join!(self.run_ttl_task(ctx.clone()), self.run_chain_watcher_task(ctx))?;
        Ok(())
    }

    async fn run_ttl_task(&self, mut ctx: ServiceContext) -> anyhow::Result<()> {
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
    use mc_db::preconfirmed::{PreconfirmedBlock, PreconfirmedExecutedTransaction};
    use mp_block::{header::PreconfirmedHeader, TransactionWithReceipt};
    use mp_receipt::{InvokeTransactionReceipt, TransactionReceipt};
    use mp_state_update::TransactionStateUpdate;
    use mp_transactions::{InvokeTransaction, Transaction};
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
            true,
        )
    }

    fn tx_account_with_nonce_and_hash(
        template: &ValidatedTransaction,
        nonce: Felt,
        hash: Felt,
    ) -> ValidatedTransaction {
        let mut tx = template.clone();
        tx.hash = hash;
        match &mut tx.transaction {
            Transaction::Invoke(InvokeTransaction::V3(inner)) => inner.nonce = nonce,
            other => panic!("unexpected transaction variant for test: {other:?}"),
        }
        tx
    }

    fn executed_preconfirmed_tx(tx: &ValidatedTransaction, resulting_nonce: Felt) -> PreconfirmedExecutedTransaction {
        PreconfirmedExecutedTransaction {
            transaction: TransactionWithReceipt {
                transaction: tx.transaction.clone(),
                receipt: TransactionReceipt::Invoke(InvokeTransactionReceipt {
                    transaction_hash: tx.hash,
                    ..Default::default()
                }),
            },
            state_diff: TransactionStateUpdate {
                nonces: [(tx.contract_address, resulting_nonce)].into(),
                ..Default::default()
            },
            declared_class: None,
            arrived_at: tx.arrived_at,
            paid_fee_on_l1: None,
        }
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

    #[rstest::rstest]
    #[timeout(Duration::from_millis(1_000))]
    #[tokio::test]
    async fn mempool_startup_reconciles_loaded_txs_against_internal_preconfirmed_runahead(
        #[future] backend: Arc<mc_db::MadaraBackend>,
        tx_account: ValidatedTransaction,
    ) {
        let backend = backend.await;

        let block_1_tx = tx_account_with_nonce_and_hash(&tx_account, Felt::ZERO, Felt::from(101u64));
        let block_2_tx = tx_account_with_nonce_and_hash(&tx_account, Felt::ONE, Felt::from(102u64));
        let stale_saved_tx = tx_account_with_nonce_and_hash(&tx_account, Felt::ONE, Felt::from(103u64));

        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new_with_content(
                PreconfirmedHeader { block_number: 1, ..Default::default() },
                [executed_preconfirmed_tx(&block_1_tx, Felt::ONE)],
                [],
            ))
            .unwrap();
        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new_with_content(
                PreconfirmedHeader { block_number: 2, ..Default::default() },
                [executed_preconfirmed_tx(&block_2_tx, Felt::from(2u64))],
                [],
            ))
            .unwrap();

        let head = backend.chain_head_state();
        assert_eq!(head.confirmed_tip, Some(0));
        assert_eq!(head.external_preconfirmed_tip, Some(1));
        assert_eq!(head.internal_preconfirmed_tip, Some(2));

        backend.write_saved_mempool_transaction(&stale_saved_tx).unwrap();
        assert_eq!(
            backend.get_saved_mempool_transactions().collect::<Result<Vec<_>, _>>().unwrap(),
            vec![stale_saved_tx.clone()]
        );

        let mempool = Mempool::new(backend.clone(), MempoolConfig::default());
        mempool.load_txs_from_db().await.unwrap();

        assert!(mempool.is_empty().await, "stale tx should be dropped during startup reconciliation");
        assert!(
            mempool.get_transaction(CONTRACT_ADDRESS, Felt::ONE, |tx| tx.hash).await.is_none(),
            "stale nonce should not remain queued after startup"
        );
        assert!(
            backend.get_saved_mempool_transactions().collect::<Result<Vec<_>, _>>().unwrap().is_empty(),
            "removed txs must also be cleared from persisted mempool storage"
        );
    }

    #[rstest::rstest]
    #[timeout(Duration::from_millis(1_000))]
    #[tokio::test]
    async fn nonce_updates_remove_stale_saved_mempool_transactions(
        #[future] backend: Arc<mc_db::MadaraBackend>,
        tx_account: ValidatedTransaction,
    ) {
        let backend = backend.await;
        let mempool = Mempool::new(backend.clone(), MempoolConfig::default());

        let queued_tx = tx_account_with_nonce_and_hash(&tx_account, Felt::ZERO, Felt::from(201u64));
        mempool.accept_tx(queued_tx.clone()).await.unwrap();

        assert_eq!(
            backend.get_saved_mempool_transactions().collect::<Result<Vec<_>, _>>().unwrap(),
            vec![queued_tx.clone()]
        );

        mempool.update_account_nonces([(queued_tx.contract_address, Felt::ONE)].into()).await.unwrap();

        assert!(mempool.is_empty().await, "nonce advancement should evict stale txs from the in-memory mempool");
        assert!(
            backend.get_saved_mempool_transactions().collect::<Result<Vec<_>, _>>().unwrap().is_empty(),
            "nonce-based removals must also clear saved mempool storage"
        );
    }

    #[rstest::rstest]
    #[timeout(Duration::from_millis(1_000))]
    #[tokio::test]
    async fn mempool_accept_writes_outbox(
        #[future] backend: Arc<mc_db::MadaraBackend>,
        tx_account: ValidatedTransaction,
    ) {
        let backend = backend.await;
        let config = MempoolConfig::default().with_external_outbox(ExternalOutboxConfig::enabled(true));
        let mempool = Mempool::new(backend.clone(), config);
        let tx = tx_account.clone();

        let result = mempool.accept_tx(tx_account).await;
        assert_matches::assert_matches!(result, Ok(()));

        let outbox: Vec<_> = backend.get_external_outbox_transactions(10).collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(outbox.len(), 1);
        assert_eq!(outbox[0].tx, tx);
    }

    #[rstest::rstest]
    #[timeout(Duration::from_millis(1_000))]
    #[tokio::test]
    async fn mempool_reject_rolls_back_outbox(
        #[future] backend: Arc<mc_db::MadaraBackend>,
        mut tx_account: ValidatedTransaction,
    ) {
        let backend = backend.await;
        let config = MempoolConfig::default().with_external_outbox(ExternalOutboxConfig::enabled(true));
        let mempool = Mempool::new(backend.clone(), config);

        tx_account.arrived_at = TxTimestamp::UNIX_EPOCH;
        let result = mempool.accept_tx(tx_account).await;
        assert_matches::assert_matches!(result, Err(MempoolInsertionError::InnerMempool(_)));

        let outbox: Vec<_> = backend.get_external_outbox_transactions(10).collect::<Result<Vec<_>, _>>().unwrap();
        assert!(outbox.is_empty());
    }

    #[rstest::rstest]
    #[timeout(Duration::from_millis(1_000))]
    #[tokio::test]
    async fn strict_outbox_rejects_on_write_failure(
        #[future] backend: Arc<mc_db::MadaraBackend>,
        tx_account: ValidatedTransaction,
    ) {
        let backend = backend.await;
        let config = MempoolConfig::default().with_external_outbox(ExternalOutboxConfig::enabled(true));
        let mempool = Mempool::new(backend.clone(), config);

        mc_db::set_external_outbox_write_failpoint(true);
        let result = mempool.accept_tx(tx_account).await;
        mc_db::set_external_outbox_write_failpoint(false);

        assert_matches::assert_matches!(result, Err(MempoolInsertionError::Internal(_)));
    }
}
