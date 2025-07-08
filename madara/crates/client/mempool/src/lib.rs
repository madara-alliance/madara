use anyhow::Context;
use async_trait::async_trait;
use mc_db::db_block_id::DbBlockId;
use mc_db::{MadaraBackend, MadaraStorageError};
use mc_submit_tx::{
    RejectedTransactionError, RejectedTransactionErrorKind, SubmitTransactionError, SubmitValidatedTransaction,
};
use metrics::MempoolMetrics;
use mp_chain_config::ChainConfig;
use mp_convert::ToFelt;
use mp_state_update::NonceUpdate;
use mp_transactions::validated::{TxTimestamp, ValidatedMempoolTx, ValidatedToBlockifierTxError};
use mp_utils::service::ServiceContext;
use notify::MempoolInnerWithNotify;
use starknet_api::core::Nonce;
use starknet_types_core::felt::Felt;
use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::Duration;

// mod inner;
mod l1;
mod new_inner;
mod notify;
use new_inner as inner;

pub use inner::*;
#[cfg(any(test, feature = "testing"))]
pub use l1::MockL1DataProvider;
pub use l1::{GasPriceProvider, L1DataProvider};
pub use notify::MempoolWriteAccess;

use crate::tx::ScoreFunction;

pub mod header;
pub mod metrics;

#[derive(thiserror::Error, Debug)]
pub enum MempoolError {
    #[error("Storage error: {0:#}")]
    StorageError(#[from] MadaraStorageError),
    #[error(transparent)]
    Internal(anyhow::Error),
    #[error(transparent)]
    InnerMempool(#[from] TxInsertionError),
    #[error("Converting validated transaction: {0:#}")]
    ValidatedToBlockifier(#[from] ValidatedToBlockifierTxError),
    #[error("Invalid nonce")]
    InvalidNonce,
}

#[derive(Debug, Clone)]
pub struct MempoolConfig {
    /// Mempool limits
    pub score_function: ScoreFunction,
    pub max_transactions: usize,
    pub max_declare_transactions: Option<usize>,
    pub ttl: Duration,
    pub no_saving: bool,
}

impl MempoolConfig {
    pub fn new(chain_config: &ChainConfig) -> Self {
        Self {
            score_function: ScoreFunction::Timestamp,
            max_declare_transactions: Some(chain_config.mempool_declare_tx_limit),
            max_transactions: chain_config.mempool_tx_limit,
            ttl: chain_config.mempool_tx_max_age.unwrap_or(Duration::MAX),
            no_saving: false,
        }
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn for_testing() -> Self {
        Self {
            score_function: ScoreFunction::Timestamp, // FCFS
            max_transactions: 10000,
            max_declare_transactions: None,
            ttl: Duration::from_secs(60 * 30),
            no_saving: false,
        }
    }

    pub fn with_no_saving(mut self, no_saving: bool) -> Self {
        self.no_saving = no_saving;
        self
    }
}

pub struct Mempool {
    backend: Arc<MadaraBackend>,
    inner: MempoolInnerWithNotify,
    metrics: MempoolMetrics,
    config: MempoolConfig,
    /// Temporary: this will move to the backend. Used for getting tx statuses.
    tx_sender: tokio::sync::broadcast::Sender<Felt>,
    /// Temporary: this will move to the backend. Used for getting tx statuses.
    received_txs: RwLock<HashSet<Felt>>,
}

impl From<MempoolError> for SubmitTransactionError {
    fn from(value: MempoolError) -> Self {
        use MempoolError as E;
        use RejectedTransactionErrorKind::*;
        use SubmitTransactionError::*;

        fn rejected(
            kind: RejectedTransactionErrorKind,
            message: impl Into<Cow<'static, str>>,
        ) -> SubmitTransactionError {
            SubmitTransactionError::Rejected(RejectedTransactionError::new(kind, message))
        }

        match value {
            err @ (E::StorageError(_) | E::ValidatedToBlockifier(_) | E::Internal(_)) => Internal(anyhow::anyhow!(err)),
            err @ E::InnerMempool(TxInsertionError::TooOld { .. }) => Internal(anyhow::anyhow!(err)),
            E::InnerMempool(TxInsertionError::DuplicateTxn) => {
                rejected(DuplicatedTransaction, "A transaction with this hash already exists in the transaction pool")
            }
            E::InnerMempool(TxInsertionError::Limit(limit)) => rejected(TransactionLimitExceeded, format!("{limit:#}")),
            E::InnerMempool(TxInsertionError::NonceConflict) => rejected(
                InvalidTransactionNonce,
                "A transaction with this nonce already exists in the transaction pool",
            ),
            E::InnerMempool(TxInsertionError::PendingDeclare) => {
                rejected(InvalidTransactionNonce, "Cannot add a declare transaction with a future nonce")
            }
            E::InnerMempool(TxInsertionError::MinTipBump { min_tip_bump }) => rejected(
                ValidateFailure,
                format!("Replacing a transaction requires at least a tip bump of at least {min_tip_bump} units"),
            ),
            E::InnerMempool(TxInsertionError::InvalidContractAddress) => {
                rejected(ValidateFailure, "Invalid contract address")
            }
            E::InnerMempool(TxInsertionError::NonceTooLow { account_nonce }) => rejected(
                InvalidTransactionNonce,
                format!("Nonce needs to be greater than the account nonce {:#x}", account_nonce.to_felt()),
            ),
            E::InnerMempool(TxInsertionError::NoTip) => {
                rejected(InvalidTransactionVersion, "Invalid transaction version")
            }
            E::InvalidNonce => rejected(InvalidTransactionNonce, "Invalid transaction nonce"),
        }
    }
}

#[async_trait]
impl SubmitValidatedTransaction for Mempool {
    async fn submit_validated_transaction(&self, tx: ValidatedMempoolTx) -> Result<(), SubmitTransactionError> {
        self.accept_tx(tx).await?;
        Ok(())
    }

    async fn received_transaction(&self, hash: Felt) -> Option<bool> {
        Some(self.received_txs.read().expect("Poisoned lock").contains(&hash))
    }

    async fn subscribe_new_transactions(&self) -> Option<tokio::sync::broadcast::Receiver<Felt>> {
        Some(self.tx_sender.subscribe())
    }
}

#[async_trait]
impl mc_submit_tx::SubmitL1HandlerTransaction for Mempool {
    async fn submit_l1_handler_transaction(
        &self,
        _tx: mp_transactions::L1HandlerTransaction,
        _paid_fees_on_l1: u128,
    ) -> Result<mp_transactions::L1HandlerTransactionResult, SubmitTransactionError> {
        unimplemented!()
    }
}

impl Mempool {
    pub fn new(backend: Arc<MadaraBackend>, config: MempoolConfig) -> Self {
        Mempool {
            backend,
            inner: MempoolInnerWithNotify::new(config.clone()),
            metrics: MempoolMetrics::register(),
            tx_sender: tokio::sync::broadcast::channel(100).0,
            config,
            received_txs: Default::default(),
        }
    }

    pub async fn load_txs_from_db(&self) -> Result<(), anyhow::Error> {
        if self.config.no_saving {
            // If saving is disabled, we don't want to read from db. Otherwise, if there are txs in the database, they will be re-inserted
            // everytime we restart the node, but will never be removed from db once they're consumed.
            return Ok(());
        }
        for res in self.backend.get_mempool_transactions() {
            let tx = res.context("Getting mempool transactions")?;
            let is_new_tx = false; // do not trigger metrics update and db update.
            if let Err(err) = self.add_tx(tx, is_new_tx).await {
                match err {
                    MempoolError::InnerMempool(TxInsertionError::TooOld { .. }) => {} // do nothing
                    err => tracing::warn!("Could not re-add mempool transaction from db: {err:#}"),
                }
            }
        }
        Ok(())
    }

    /// Accept a new transaction.
    async fn accept_tx(&self, tx: ValidatedMempoolTx) -> Result<(), MempoolError> {
        self.add_tx(tx, /* is_new_tx */ true).await
    }

    /// Use `is_new_tx: false` when loading transactions from db, so that we skip saving in db and updating metrics.
    async fn add_tx(&self, tx: ValidatedMempoolTx, is_new_tx: bool) -> Result<(), MempoolError> {
        tracing::debug!("Accepting transaction tx_hash={:#x} is_new_tx={is_new_tx}", tx.tx_hash);

        let now = TxTimestamp::now();
        let account_nonce =
            self.backend.get_contract_nonce_at(&DbBlockId::Pending, &tx.contract_address)?.unwrap_or(Felt::ZERO);
        let mut removed_txs = smallvec::SmallVec::<[ValidatedMempoolTx; 1]>::new();
        // Lock is acquired here and dropped immediately after.
        let ret = self.inner.write().await.insert_tx(now, tx.clone(), Nonce(account_nonce), &mut removed_txs);
        self.on_txs_removed(&removed_txs);
        if ret.is_ok() {
            self.on_tx_added(&tx, is_new_tx);
        }
        ret.map_err(Into::into)
    }

    /// Update secondary state when a new transaction has been successfully added to the mempool.
    /// Use `is_new_tx: true` when loading transactions from db, so that we skip saving in db and updating metrics.
    fn on_tx_added(&self, tx: &ValidatedMempoolTx, is_new_tx: bool) {
        tracing::debug!("Accepted transaction tx_hash={:#x}", tx.tx_hash);
        if is_new_tx {
            self.metrics.accepted_transaction_counter.add(1, &[]);
            if !self.config.no_saving {
                if let Err(err) = self.backend.save_mempool_transaction(tx) {
                    tracing::error!("Could not add mempool transaction to database: {err:#}");
                }
            }
        }
        self.received_txs.write().expect("Poisoned lock").insert(tx.tx_hash);
        let _ = self.tx_sender.send(tx.tx_hash);
    }

    /// Update secondary state when a new transaction has been successfully removed from the mempool.
    fn on_txs_removed(&self, removed: &[ValidatedMempoolTx]) {
        if !self.config.no_saving {
            if let Err(err) = self.backend.remove_mempool_transactions(removed.iter().map(|tx| tx.tx_hash)) {
                tracing::error!("Could not remove mempool transactions from database: {err:#}");
            }
        }
        let mut lock = self.received_txs.write().expect("Poisoned lock");
        for tx in removed {
            tracing::debug!("Removed transaction tx_hash={:#x}", tx.tx_hash);
            lock.remove(&tx.tx_hash);
        }
        // TODO: tell self.tx_sender about the removal
    }

    /// Update secondary state when a new transaction has been consumed for block production.
    fn on_tx_consumed(&self, _removed: &ValidatedMempoolTx) {
        // nothing here yet.
    }

    /// Temporary: this will move to the backend. Called by block production & locally when txs are added to the chain.
    fn remove_from_received(&self, txs: &[Felt]) {
        if !self.config.no_saving {
            if let Err(err) = self.backend.remove_mempool_transactions(txs.iter().copied()) {
                tracing::error!("Could not remove mempool transactions from database: {err:#}");
            }
            // TODO: tell self.tx_sender about the removal
        }
        let mut lock = self.received_txs.write().expect("Poisoned lock");
        for tx in txs {
            lock.remove(&tx);
        }
    }

    /// This is called directly by the block production task for now.
    pub async fn on_tx_batch_executed(
        &self,
        new_nonce_updates: impl IntoIterator<Item = NonceUpdate>,
        executed_txs: impl IntoIterator<Item = Felt>,
    ) -> anyhow::Result<()> {
        let updates = new_nonce_updates
            .into_iter()
            .map(|el| Ok((el.contract_address.try_into()?, Nonce(el.nonce))))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let executed_txs = executed_txs.into_iter().collect::<Vec<_>>();

        let mut removed_txs = smallvec::SmallVec::<[ValidatedMempoolTx; 1]>::new();
        {
            let mut lock = self.inner.write().await;
            for (contract_address, nonce) in updates {
                lock.update_account_nonce(&contract_address, &nonce, &mut removed_txs);
            }
        }
        self.on_txs_removed(&removed_txs);

        self.remove_from_received(&executed_txs);

        Ok(())
    }

    async fn remove_ttl_exceeded_txs(&self) -> anyhow::Result<()> {
        let mut removed_txs = smallvec::SmallVec::<[ValidatedMempoolTx; 1]>::new();
        let now = TxTimestamp::now();
        {
            let mut lock = self.inner.write().await;
            lock.remove_all_ttl_exceeded_txs(now, &mut removed_txs);
        }
        self.on_txs_removed(&removed_txs);
        Ok(())
    }

    pub async fn run_mempool_task(&self, mut ctx: ServiceContext) -> anyhow::Result<()> {
        self.load_txs_from_db().await.context("Loading transactions from db on mempool startup.")?;
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
        f: impl FnOnce(&ValidatedMempoolTx) -> R,
    ) -> Option<R> {
        let lock = self.inner.read().await;
        lock.get_transaction(&contract_address.try_into().ok()?, &Nonce(nonce)).map(f)
    }

    #[tracing::instrument(skip(self), fields(module = "Mempool"))]
    /// Returns a view of the mempool intended for consuming transactions from the mempool.
    /// If the mempool has no mempool that can be consumed, this function will wait until there is at least 1 transaction to consume.
    /// This holds the lock to the inner mempool - use with care.
    pub async fn get_consumer(&self) -> MempoolConsumer<'_> {
        MempoolConsumer { lock: self.inner.get_write_access_wait_for_ready().await, mempool: self }
    }
}

/// A view into the mempool, intended for consuming transactions. This is expected to be used by block production to
/// pop transactions from the mempool and execute them.
///
/// This struct implements [`Iterator`] by popping the next transaction to execute from the mempool.
///
/// This holds the lock to the inner mempool - use with care.
pub struct MempoolConsumer<'a> {
    lock: MempoolWriteAccess<'a>,
    mempool: &'a Mempool,
}
impl Iterator for MempoolConsumer<'_> {
    type Item = ValidatedMempoolTx;
    fn next(&mut self) -> Option<Self::Item> {
        self.lock.pop_next_ready().inspect(|tx| self.mempool.on_tx_consumed(tx))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let n_ready = self.lock.ready_transactions();
        (n_ready, Some(n_ready))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use starknet_api::core::ContractAddress;
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
    pub fn tx_account_v0_valid(#[default(CONTRACT_ADDRESS)] contract_address: Felt) -> ValidatedMempoolTx {
        static HASH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

        let ordering = std::sync::atomic::Ordering::AcqRel;
        let tx_hash = starknet_api::transaction::TransactionHash(HASH.fetch_add(1, ordering).into());

        ValidatedMempoolTx::from_blockifier(
            blockifier::transaction::transaction_execution::Transaction::AccountTransaction(
                blockifier::transaction::account_transaction::AccountTransaction::Invoke(
                    blockifier::transaction::transactions::InvokeTransaction {
                        tx: starknet_api::transaction::InvokeTransaction::V0(
                            starknet_api::transaction::InvokeTransactionV0 {
                                contract_address: ContractAddress::try_from(contract_address).unwrap(),
                                ..Default::default()
                            },
                        ),
                        tx_hash,
                        only_query: false,
                    },
                ),
            ),
            TxTimestamp::now(),
            None,
        )
    }

    #[rstest::fixture]
    pub fn tx_account_v1_invalid() -> ValidatedMempoolTx {
        ValidatedMempoolTx::from_blockifier(
            blockifier::transaction::transaction_execution::Transaction::AccountTransaction(
                blockifier::transaction::account_transaction::AccountTransaction::Invoke(
                    blockifier::transaction::transactions::InvokeTransaction {
                        tx: starknet_api::transaction::InvokeTransaction::V1(
                            starknet_api::transaction::InvokeTransactionV1::default(),
                        ),
                        tx_hash: starknet_api::transaction::TransactionHash::default(),
                        only_query: true,
                    },
                ),
            ),
            TxTimestamp::now(),
            None,
        )
    }

    #[rstest::fixture]
    pub fn tx_deploy_v1_valid(#[default(CONTRACT_ADDRESS)] contract_address: Felt) -> ValidatedMempoolTx {
        ValidatedMempoolTx::from_blockifier(
            blockifier::transaction::transaction_execution::Transaction::AccountTransaction(
                blockifier::transaction::account_transaction::AccountTransaction::DeployAccount(
                    blockifier::transaction::transactions::DeployAccountTransaction {
                        tx: starknet_api::transaction::DeployAccountTransaction::V1(
                            starknet_api::transaction::DeployAccountTransactionV1::default(),
                        ),
                        tx_hash: starknet_api::transaction::TransactionHash::default(),
                        contract_address: ContractAddress::try_from(contract_address).unwrap(),
                        only_query: false,
                    },
                ),
            ),
            TxTimestamp::now(),
            None,
        )
    }

    #[rstest::fixture]
    fn tx_l1_handler_valid(#[default(CONTRACT_ADDRESS)] contract_address: Felt) -> ValidatedMempoolTx {
        ValidatedMempoolTx::from_blockifier(
            blockifier::transaction::transaction_execution::Transaction::L1HandlerTransaction(
                blockifier::transaction::transactions::L1HandlerTransaction {
                    tx: starknet_api::transaction::L1HandlerTransaction {
                        contract_address: ContractAddress::try_from(contract_address).unwrap(),
                        ..Default::default()
                    },
                    tx_hash: starknet_api::transaction::TransactionHash::default(),
                    paid_fee_on_l1: starknet_api::transaction::Fee::default(),
                },
            ),
            TxTimestamp::now(),
            None,
        )
    }

    #[rstest::rstest]
    #[timeout(Duration::from_millis(1_000))]
    #[tokio::test]
    async fn mempool_accept_tx_pass(
        #[future] backend: Arc<mc_db::MadaraBackend>,
        tx_account_v0_valid: ValidatedMempoolTx,
    ) {
        let backend = backend.await;
        let mempool = Mempool::new(backend, MempoolConfig::for_testing());
        let result = mempool.accept_tx(tx_account_v0_valid).await;
        assert_matches::assert_matches!(result, Ok(()));

        mempool.inner.read().await.check_invariants();
    }

    /// This test makes sure that taking a transaction from the mempool works as
    /// intended.
    #[rstest::rstest]
    #[timeout(Duration::from_millis(1_000))]
    #[tokio::test]
    async fn mempool_take_tx_pass(
        #[future] backend: Arc<mc_db::MadaraBackend>,
        mut tx_account_v0_valid: ValidatedMempoolTx,
    ) {
        let backend = backend.await;
        let mempool = Mempool::new(backend, MempoolConfig::for_testing());
        let timestamp = TxTimestamp::now();
        tx_account_v0_valid.arrived_at = timestamp;
        let result = mempool.accept_tx(tx_account_v0_valid).await;
        assert_matches::assert_matches!(result, Ok(()));

        let mempool_tx = mempool.get_consumer().await.next().expect("Mempool should contain a transaction");
        assert_eq!(mempool_tx.arrived_at, timestamp);

        assert!(
            mempool.get_consumer().await.next().is_none(),
            "It should not be possible to take a transaction from an empty mempool"
        );

        mempool.inner.read().await.check_invariants();
    }
}
