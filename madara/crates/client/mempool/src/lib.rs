use anyhow::Context;
use async_trait::async_trait;
use blockifier::transaction::transaction_execution::Transaction;
use mc_db::mempool_db::{DbMempoolTxInfoDecoder, NonceInfo};
use mc_db::{MadaraBackend, MadaraStorageError};
use mc_exec::execution::TxInfo;
use mc_submit_tx::{
    RejectedTransactionError, RejectedTransactionErrorKind, SubmitL1HandlerTransaction, SubmitTransactionError,
    SubmitValidatedTransaction,
};
use metrics::MempoolMetrics;
use mp_block::{BlockId, BlockTag};
use mp_class::ConvertedClass;
use mp_convert::ToFelt;
use mp_transactions::validated::{TxTimestamp, ValidatedMempoolTx, ValidatedToBlockifierTxError};
use mp_transactions::L1HandlerTransaction;
use mp_transactions::L1HandlerTransactionResult;
use notify::MempoolInnerWithNotify;
use starknet_api::core::Nonce;
use starknet_api::transaction::TransactionVersion;
use starknet_types_core::felt::Felt;
use std::borrow::Cow;
use std::sync::Arc;

mod inner;
mod l1;
mod notify;

pub use inner::*;
#[cfg(any(test, feature = "testing"))]
pub use l1::MockL1DataProvider;
pub use l1::{GasPriceProvider, L1DataProvider};
pub use notify::MempoolConsumerView;

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

#[cfg(any(test, feature = "testing"))]
#[allow(unused)]
pub(crate) trait CheckInvariants {
    /// Validates the invariants of this struct.
    ///
    /// This method should cause a panic if these invariants are not met.
    fn check_invariants(&self);
}

#[derive(Debug, Clone)]
pub struct MempoolConfig {
    /// Mempool limits
    pub limits: MempoolLimits,
    pub no_saving: bool,
}

impl MempoolConfig {
    pub fn new(limits: MempoolLimits) -> Self {
        Self { limits, no_saving: false }
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn for_testing() -> Self {
        Self::new(MempoolLimits::for_testing())
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
    tx_sender: tokio::sync::broadcast::Sender<Felt>,
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
            E::InnerMempool(TxInsertionError::DuplicateTxn) => {
                rejected(DuplicatedTransaction, "A transaction with this hash already exists in the transaction pool")
            }
            E::InnerMempool(TxInsertionError::Limit(limit)) => rejected(TransactionLimitExceeded, format!("{limit:#}")),
            E::InnerMempool(TxInsertionError::NonceConflict) => rejected(
                InvalidTransactionNonce,
                "A transaction with this nonce already exists in the transaction pool",
            ),
            E::InvalidNonce => rejected(InvalidTransactionNonce, "Invalid transaction nonce"),
        }
    }
}

#[async_trait]
impl SubmitValidatedTransaction for Mempool {
    async fn submit_validated_transaction(&self, tx: ValidatedMempoolTx) -> Result<(), SubmitTransactionError> {
        let tx_hash = tx.tx_hash;
        self.accept_tx(tx).await?;
        let _ = self.tx_sender.send(tx_hash);
        Ok(())
    }

    async fn received_transaction(&self, hash: mp_convert::Felt) -> Option<bool> {
        Some(self.inner.read().await.has_transaction(&starknet_api::transaction::TransactionHash(hash)))
    }

    async fn subscribe_new_transactions(&self) -> Option<tokio::sync::broadcast::Receiver<mp_convert::Felt>> {
        Some(self.tx_sender.subscribe())
    }
}

#[async_trait]
impl SubmitL1HandlerTransaction for Mempool {
    async fn submit_l1_handler_transaction(
        &self,
        tx: L1HandlerTransaction,
        paid_fees_on_l1: u128,
    ) -> Result<L1HandlerTransactionResult, SubmitTransactionError> {
        let arrived_at = TxTimestamp::now();
        let (tx, converted_class) = tx
            .into_blockifier(
                self.backend.chain_config().chain_id.to_felt(),
                self.backend.chain_config().latest_protocol_version,
                paid_fees_on_l1,
            )
            .context("Converting l1 handler tx to blockifier")
            .map_err(SubmitTransactionError::Internal)?;
        let res = L1HandlerTransactionResult { transaction_hash: tx.tx_hash().to_felt() };
        self.accept_tx(ValidatedMempoolTx::from_blockifier(tx, arrived_at, converted_class)).await?;
        Ok(res)
    }
}

impl Mempool {
    pub fn new(backend: Arc<MadaraBackend>, config: MempoolConfig) -> Self {
        Mempool {
            backend,
            inner: MempoolInnerWithNotify::new(config.limits.clone()),
            metrics: MempoolMetrics::register(),
            tx_sender: tokio::sync::broadcast::channel(100).0,
            config,
        }
    }

    pub async fn load_txs_from_db(&mut self) -> Result<(), anyhow::Error> {
        for res in self.backend.get_mempool_transactions() {
            let (_, DbMempoolTxInfoDecoder { tx, nonce_readiness }) = res.context("Getting mempool transactions")?;

            let tx_hash = tx.tx_hash;
            let (tx, arrived_at, converted_class) = tx
                .into_blockifier()
                .context("Converting validated tx to blockifier")
                .map_err(SubmitTransactionError::Internal)?;

            if let Err(err) = self.add_to_inner_mempool(tx_hash, tx, arrived_at, converted_class, nonce_readiness).await
            {
                match err {
                    MempoolError::InnerMempool(TxInsertionError::Limit(MempoolLimitReached::Age { .. })) => {} // do nothing
                    err => tracing::warn!("Could not re-add mempool transaction from db: {err:#}"),
                }
            }
        }
        Ok(())
    }

    async fn accept_tx_with_nonce_info(
        &self,
        tx: ValidatedMempoolTx,
        nonce_info: NonceInfo,
    ) -> Result<(), MempoolError> {
        // TODO: should we update this to store only if the mempool accepts
        // this transaction?
        if !self.config.no_saving {
            self.backend.save_mempool_transaction(&tx, &nonce_info).map_err(MempoolError::from)?;
        }

        let tx_hash = tx.tx_hash;
        let (tx, arrived_at, converted_class) = tx.into_blockifier()?;

        self.add_to_inner_mempool(tx_hash, tx, arrived_at, converted_class, nonce_info).await?;

        Ok(())
    }

    async fn accept_tx(&self, tx: ValidatedMempoolTx) -> Result<(), MempoolError> {
        let nonce_info = if tx.tx.version() == TransactionVersion::ZERO {
            NonceInfo::default()
        } else if let Some(tx) = tx.tx.as_l1_handler() {
            self.resolve_nonce_info_l1_handler(tx.nonce.into())?
        } else {
            self.retrieve_nonce_info(tx.contract_address, tx.tx.nonce()).await?
        };
        self.accept_tx_with_nonce_info(tx, nonce_info).await
    }

    /// Does not save to the database.
    async fn add_to_inner_mempool(
        &self,
        tx_hash: Felt,
        tx: Transaction,
        arrived_at: TxTimestamp,
        converted_class: Option<ConvertedClass>,
        nonce_info: NonceInfo,
    ) -> Result<(), MempoolError> {
        tracing::debug!("Adding to inner mempool tx_hash={:#x}", tx_hash);

        // Add it to the inner mempool
        let force = false;
        let nonce = nonce_info.nonce;
        let nonce_next = nonce_info.nonce_next;
        self.inner
            .insert_tx(
                MempoolTransaction { tx, arrived_at, converted_class, nonce, nonce_next },
                force,
                /* update_limits */ true,
                nonce_info,
            )
            .await?;

        self.metrics.accepted_transaction_counter.add(1, &[]);

        Ok(())
    }

    #[cfg(any(test, feature = "testing"))]
    pub async fn is_empty(&self) -> bool {
        self.inner.read().await.is_empty()
    }

    /// Determines the status of a transaction based on the address of the
    /// contract sending it and its nonce.
    ///
    /// Several mechanisms are used to keep the latest nonce for a contract in
    /// sync with the state of the node, even if the db is not being updated
    /// fast enough. These include keeping a mapping of the latest nonce for
    /// contracts to be included in the upcoming block as well as checking the
    /// state of the mempool to see if previous transactions are marked as
    /// ready.
    async fn retrieve_nonce_info(&self, sender_address: Felt, nonce: Felt) -> Result<NonceInfo, MempoolError> {
        let nonce = Nonce(nonce);
        let nonce_next = nonce.try_increment().context("Nonce overflow").map_err(MempoolError::Internal)?;

        let nonce_prev_check = {
            // We don't need an underflow check here as nonces are incremental
            // and non negative, so there is no nonce s.t nonce != nonce_target,
            // nonce < nonce_target & nonce = 0
            let nonce_prev = Nonce(nonce.0 - Felt::ONE);
            let nonce_prev_ready = self.inner.read().await.nonce_is_ready(sender_address, nonce_prev);

            // If the mempool has the transaction before this one ready, then
            // this transaction is ready too. Even if the db has not been
            // updated yet, this transaction will always be polled and executed
            // afterwards.
            if nonce_prev_ready {
                Ok::<_, MempoolError>(NonceInfo::ready(nonce, nonce_next))
            } else {
                Ok::<_, MempoolError>(NonceInfo::pending(nonce, nonce_next))
            }
        };

        // It is possible for a transaction to be polled, and the
        // transaction right after it to be added into the mempoool, before
        // the db is updated. For this reason, nonce_cache keeps track of
        // the nonces of contracts which have not yet been included in the
        // block but are scheduled to.
        let nonce_cached = self.inner.nonce_cache_read().await.get(&sender_address).cloned();

        if let Some(nonce_cached) = nonce_cached {
            match nonce.cmp(&nonce_cached) {
                std::cmp::Ordering::Less => Err(MempoolError::InvalidNonce),
                std::cmp::Ordering::Equal => Ok(NonceInfo::ready(nonce, nonce_next)),
                std::cmp::Ordering::Greater => nonce_prev_check,
            }
        } else {
            // The nonce cache avoids us a db lookup if the previous transaction
            // is already scheduled for inclusion in this block.
            let nonce_target = self
                .backend
                .get_contract_nonce_at(&BlockId::Tag(BlockTag::Latest), &sender_address)?
                .map(Nonce)
                .unwrap_or_default(); // Defaults to Felt::ZERO if no nonce in db

            match nonce.cmp(&nonce_target) {
                std::cmp::Ordering::Less => Err(MempoolError::InvalidNonce),
                std::cmp::Ordering::Equal => Ok(NonceInfo::ready(nonce, nonce_next)),
                std::cmp::Ordering::Greater => nonce_prev_check,
            }
        }
    }

    /// This function determines the Nonce status (NonceInfo) for incoming L1 transactions
    /// based on the last processed nonce (current_nonce) in the system.
    ///
    /// L1 Handler nonces represent the ordering of L1 transactions sent by the
    /// core L1 contract. In principle this is a bit strange, as there currently
    /// is only 1 core L1 contract, so all transactions should be ordered by
    /// default. Moreover, these transaction are infrequent, so the risk that
    /// two transactions are emitted at very short intervals seems unlikely.
    /// Still, who knows?
    fn resolve_nonce_info_l1_handler(&self, nonce: Felt) -> Result<NonceInfo, MempoolError> {
        let nonce = Nonce(nonce);
        let nonce_next = nonce.try_increment().context("Nonce overflow").map_err(MempoolError::Internal)?;

        // TODO: This would break if the txs are not ordered --> l1 nonce latest should be updated only after execution
        // Currently is updated after inclusion in mempool
        let current_nonce = self.backend.get_l1_messaging_nonce_latest()?;
        // first l1 handler tx, where get_l1_messaging_nonce_latest returns None
        let target_nonce = match current_nonce {
            Some(nonce) => nonce.try_increment().context("Nonce overflow").map_err(MempoolError::Internal)?,
            None => Nonce(Felt::ZERO),
        };

        match nonce.cmp(&target_nonce) {
            std::cmp::Ordering::Less => Err(MempoolError::InvalidNonce),
            std::cmp::Ordering::Equal => Ok(NonceInfo::ready(nonce, nonce_next)),
            std::cmp::Ordering::Greater => Ok(NonceInfo::pending(nonce, nonce_next)),
        }
    }

    #[tracing::instrument(skip(self), fields(module = "Mempool"))]
    /// Returns a view of the mempool intended for consuming transactions from the mempool.
    /// If the mempool has no mempool that can be consumed, this function will wait until there is at least 1 transaction to consume.
    pub async fn get_consumer_wait_for_ready_tx(&self) -> MempoolConsumerView<'_> {
        self.inner.get_consumer_wait_for_ready_tx().await
    }

    #[tracing::instrument(skip(self), fields(module = "Mempool"))]
    /// Returns a view of the mempool intended for consuming transactions from the mempool.
    pub async fn get_consumer(&self) -> MempoolConsumerView<'_> {
        self.inner.get_consumer().await
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use mc_db::mempool_db::NonceStatus;
    use mp_block::{MadaraBlockInfo, MadaraBlockInner, MadaraMaybePendingBlock, MadaraMaybePendingBlockInfo};
    use mp_state_update::{NonceUpdate, StateDiff};
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

    /// This test checks if a ready transaction is indeed inserted into the
    /// ready queue.
    #[rstest::rstest]
    #[timeout(Duration::from_millis(1_000))]
    #[tokio::test]
    async fn mempool_accept_tx_ready(
        #[future] backend: Arc<mc_db::MadaraBackend>,
        tx_account_v0_valid: ValidatedMempoolTx,
    ) {
        let backend = backend.await;

        let nonce = Nonce(Felt::ZERO);

        let mempool = Mempool::new(backend, MempoolConfig::for_testing());
        let arrived_at = tx_account_v0_valid.arrived_at;
        let result = mempool.accept_tx(tx_account_v0_valid).await;
        assert_matches::assert_matches!(result, Ok(()));

        let inner = mempool.inner.read().await;
        inner.check_invariants();

        assert!(inner.tx_intent_queue_ready.contains(&TransactionIntentReady {
            contract_address: CONTRACT_ADDRESS,
            timestamp: arrived_at,
            nonce: Nonce(Felt::ZERO),
            nonce_next: Nonce(Felt::ONE),
            phantom: std::marker::PhantomData,
        }),);

        assert!(inner.nonce_is_ready(CONTRACT_ADDRESS, nonce));
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

    /// This test makes sure that all deploy account transactions inserted into
    /// [MempoolInner] are accounted for. Replacements are not taken into
    /// account.
    #[rstest::rstest]
    #[timeout(Duration::from_millis(1_000))]
    #[tokio::test]
    async fn mempool_deploy_count(
        #[future] backend: Arc<mc_db::MadaraBackend>,
        tx_deploy_v1_valid: ValidatedMempoolTx,
    ) {
        let backend = backend.await;
        let mempool = Mempool::new(backend, MempoolConfig::for_testing());

        let nonce_info = NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE));
        let mempool_tx = MempoolTransaction {
            tx: tx_deploy_v1_valid.into_blockifier().unwrap().0,
            arrived_at: TxTimestamp::now(),
            converted_class: None,
            nonce: nonce_info.nonce,
            nonce_next: nonce_info.nonce_next,
        };
        let contract_address = mempool_tx.contract_address();

        let force = true;
        let update_limits = true;
        let result = mempool.inner.insert_tx(mempool_tx, force, update_limits, nonce_info).await;
        assert_matches::assert_matches!(result, Ok(()));

        let inner = mempool.inner.read().await;
        assert!(inner.deployed_contracts.contains(&contract_address));
        inner.check_invariants();
    }

    /// This test makes sure that all deploy account transactions inserted into
    /// [MempoolInner] are accounted for, even after a non-deploy transaction
    /// has been replaced.
    ///
    /// > This bug was originally detected through proptesting.
    #[rstest::rstest]
    #[timeout(Duration::from_millis(1_000))]
    #[tokio::test]
    async fn mempool_deploy_replace(
        #[future] backend: Arc<mc_db::MadaraBackend>,
        tx_account_v0_valid: ValidatedMempoolTx,
        tx_deploy_v1_valid: ValidatedMempoolTx,
    ) {
        let backend = backend.await;
        let mempool = Mempool::new(backend, MempoolConfig::for_testing());

        let nonce_info = NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE));
        let mempool_tx = MempoolTransaction {
            tx: tx_account_v0_valid.into_blockifier().unwrap().0,
            arrived_at: TxTimestamp::now(),
            converted_class: None,
            nonce: nonce_info.nonce,
            nonce_next: nonce_info.nonce_next,
        };
        let contract_address = mempool_tx.contract_address();

        // We hack our way into the inner mempool to avoid having to generate a
        // valid deploy transaction since the outer mempool checks this
        let force = false;
        let update_limits = true;
        let result = mempool.inner.insert_tx(mempool_tx, force, update_limits, nonce_info).await;
        assert_matches::assert_matches!(result, Ok(()));

        // We insert a first non-deploy tx. This should not update the count of
        // deploy transactions.
        let inner = mempool.inner.read().await;
        assert!(!inner.deployed_contracts.contains(&contract_address));
        inner.check_invariants();
        drop(inner);

        let nonce_info = NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE));
        let mempool_tx = MempoolTransaction {
            tx: tx_deploy_v1_valid.into_blockifier().unwrap().0,
            arrived_at: TxTimestamp::now(),
            converted_class: None,
            nonce: nonce_info.nonce,
            nonce_next: nonce_info.nonce_next,
        };
        let contract_address = mempool_tx.contract_address();

        // Now we replace the previous transaction with a deploy account tx
        let force = true;
        let update_limits = true;
        let result = mempool.inner.insert_tx(mempool_tx, force, update_limits, nonce_info).await;
        assert_matches::assert_matches!(result, Ok(()));

        // This should have updated the count of deploy transactions.
        let inner = mempool.inner.read().await;
        assert!(inner.deployed_contracts.contains(&contract_address));
        inner.check_invariants();
    }

    /// This test makes sure that replacing a deploy account transaction with a
    /// non-deploy account transaction reduces the deploy transaction count.
    ///
    /// > This bug was originally detected through proptesting
    #[rstest::rstest]
    #[timeout(Duration::from_millis(1_000))]
    #[tokio::test]
    async fn mempool_deploy_replace2(
        #[future] backend: Arc<mc_db::MadaraBackend>,
        tx_deploy_v1_valid: ValidatedMempoolTx,
        tx_account_v0_valid: ValidatedMempoolTx,
    ) {
        let backend = backend.await;
        let mempool = Mempool::new(backend, MempoolConfig::for_testing());

        // First, we insert the deploy account transaction
        let nonce_info = NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE));
        let mempool_tx = MempoolTransaction {
            tx: tx_deploy_v1_valid.into_blockifier().unwrap().0,
            arrived_at: TxTimestamp::now(),
            converted_class: None,
            nonce: nonce_info.nonce,
            nonce_next: nonce_info.nonce_next,
        };
        let contract_address = mempool_tx.contract_address();

        let force = true;
        let update_limits = true;
        let result = mempool.inner.insert_tx(mempool_tx, force, update_limits, nonce_info).await;
        assert_matches::assert_matches!(result, Ok(()));

        let inner = mempool.inner.read().await;
        assert!(inner.deployed_contracts.contains(&contract_address));
        inner.check_invariants();
        drop(inner);

        // Now we replace the previous transaction with a non-deploy account tx
        let nonce_info = NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE));
        let mempool_tx = MempoolTransaction {
            tx: tx_account_v0_valid.into_blockifier().unwrap().0,
            arrived_at: TxTimestamp::now(),
            converted_class: None,
            nonce: nonce_info.nonce,
            nonce_next: nonce_info.nonce_next,
        };
        let contract_address = mempool_tx.contract_address();

        let force = true;
        let update_limits = true;
        let result = mempool.inner.insert_tx(mempool_tx, force, update_limits, nonce_info).await;
        assert_matches::assert_matches!(result, Ok(()));

        // The deploy transaction count at that address should be 0
        let inner = mempool.inner.read().await;
        assert!(!inner.deployed_contracts.contains(&contract_address));
        inner.check_invariants();
    }

    /// This tests makes sure that when deploy account transactions are removed
    /// because their age exceeds the allowed limit, then the deploy transaction
    /// count is also updated.
    ///
    /// > This bug was originally detected through proptesting
    #[rstest::rstest]
    #[timeout(Duration::from_millis(1_000))]
    #[tokio::test]
    async fn mempool_deploy_remove_age_exceeded(
        #[future] backend: Arc<mc_db::MadaraBackend>,
        tx_deploy_v1_valid: ValidatedMempoolTx,
    ) {
        let backend = backend.await;
        let mempool = Mempool::new(
            backend,
            MempoolConfig::new(MempoolLimits {
                max_age: Some(Duration::from_secs(3_600)),
                ..MempoolLimits::for_testing()
            }),
        );

        // First, we insert the deploy account transaction
        let nonce_info = NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE));
        let mempool_tx = MempoolTransaction {
            tx: tx_deploy_v1_valid.into_blockifier().unwrap().0,
            arrived_at: TxTimestamp::UNIX_EPOCH,
            converted_class: None,
            nonce: nonce_info.nonce,
            nonce_next: nonce_info.nonce_next,
        };
        let contract_address = mempool_tx.contract_address();

        let force = true;
        let update_limits = true;
        let result = mempool.inner.insert_tx(mempool_tx, force, update_limits, nonce_info).await;
        assert_matches::assert_matches!(result, Ok(()));

        let inner = mempool.inner.read().await;
        assert!(inner.deployed_contracts.contains(&contract_address));
        inner.check_invariants();
        drop(inner);

        // Next, we manually call `remove_age_exceeded_txs`
        mempool.inner.write().await.remove_age_exceeded_txs();

        // This should have removed the deploy contract transaction and updated
        // the count at that address
        let inner = mempool.inner.read().await;
        assert!(!inner.deployed_contracts.contains(&contract_address));
        inner.check_invariants();
    }

    /// This test makes sure that old transactions are removed from the
    /// [mempool], whether they be represented by ready or pending intents.
    ///
    /// # Setup:
    ///
    /// - We assume `tx_*_n `are from the same contract but with increasing
    ///   nonces.
    ///
    /// - `tx_new` are transactions which _should not_ be removed from the
    ///   mempool, `tx_old` are transactions which _should_ be removed from the
    ///   mempool
    ///
    /// - `tx_new_3`, `tx_old_3` and `tx_new_2_bis` and `tx_old_4` are in the
    ///   pending queue, all other transactions are in the ready queue.
    ///
    /// [mempool]: inner::MempoolInner
    #[rstest::rstest]
    #[timeout(Duration::from_millis(1_000))]
    #[allow(clippy::too_many_arguments)]
    #[tokio::test]
    async fn mempool_remove_aged_tx_pass(
        #[future] backend: Arc<mc_db::MadaraBackend>,
        #[from(tx_account_v0_valid)] // We are reusing the tx_account_v0_valid fixture...
        #[with(Felt::ZERO)] // ...with different arguments
        tx_new_1: ValidatedMempoolTx,
        #[from(tx_account_v0_valid)]
        #[with(Felt::ONE)]
        tx_new_2: ValidatedMempoolTx,
        #[from(tx_account_v0_valid)]
        #[with(Felt::TWO)]
        tx_new_3: ValidatedMempoolTx,
        #[from(tx_account_v0_valid)]
        #[with(Felt::ZERO)]
        tx_old_1: ValidatedMempoolTx,
        #[from(tx_account_v0_valid)]
        #[with(Felt::ONE)]
        tx_old_2: ValidatedMempoolTx,
        #[from(tx_account_v0_valid)]
        #[with(Felt::TWO)]
        tx_old_3: ValidatedMempoolTx,
        #[from(tx_account_v0_valid)]
        #[with(Felt::THREE)]
        tx_old_4: ValidatedMempoolTx,
    ) {
        let backend = backend.await;
        let mempool = Mempool::new(
            backend,
            MempoolConfig::new(MempoolLimits {
                max_age: Some(Duration::from_secs(3600)),
                ..MempoolLimits::for_testing()
            }),
        );

        // ================================================================== //
        //                                STEP 1                              //
        // ================================================================== //

        // First, we begin by inserting all our transactions and making sure
        // they are in the ready as well as the pending intent queues.
        let arrived_at = TxTimestamp::now();
        let tx_new_1_mempool = MempoolTransaction {
            tx: tx_new_1.into_blockifier().unwrap().0,
            arrived_at,
            converted_class: None,
            nonce: Nonce(Felt::ZERO),
            nonce_next: Nonce(Felt::ONE),
        };
        let res = mempool
            .inner
            .insert_tx(tx_new_1_mempool.clone(), true, false, NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE)))
            .await;
        assert!(res.is_ok());
        assert!(
            mempool.inner.read().await.tx_intent_queue_ready.contains(&TransactionIntentReady {
                contract_address: **tx_new_1_mempool.contract_address(),
                timestamp: tx_new_1_mempool.arrived_at,
                nonce: tx_new_1_mempool.nonce,
                nonce_next: tx_new_1_mempool.nonce_next,
                phantom: std::marker::PhantomData
            }),
            "ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().await.tx_intent_queue_ready,
            mempool.inner.read().await.tx_intent_queue_pending_by_nonce
        );

        let arrived_at = TxTimestamp::now();
        let tx_new_2_mempool = MempoolTransaction {
            tx: tx_new_2.into_blockifier().unwrap().0,
            arrived_at,
            converted_class: None,
            nonce: Nonce(Felt::ZERO),
            nonce_next: Nonce(Felt::ONE),
        };
        let res = mempool
            .inner
            .insert_tx(tx_new_2_mempool.clone(), true, false, NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE)))
            .await;
        assert!(res.is_ok());
        assert!(
            mempool.inner.read().await.tx_intent_queue_ready.contains(&TransactionIntentReady {
                contract_address: **tx_new_2_mempool.contract_address(),
                timestamp: tx_new_2_mempool.arrived_at,
                nonce: tx_new_2_mempool.nonce,
                nonce_next: tx_new_2_mempool.nonce_next,
                phantom: std::marker::PhantomData
            }),
            "ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().await.tx_intent_queue_ready,
            mempool.inner.read().await.tx_intent_queue_pending_by_nonce
        );

        let arrived_at = TxTimestamp::now();
        let tx_new_3_mempool = MempoolTransaction {
            tx: tx_new_3.into_blockifier().unwrap().0,
            arrived_at,
            converted_class: None,
            nonce: Nonce(Felt::ONE),
            nonce_next: Nonce(Felt::TWO),
        };
        let res = mempool
            .inner
            .insert_tx(tx_new_3_mempool.clone(), true, false, NonceInfo::pending(Nonce(Felt::ONE), Nonce(Felt::TWO)))
            .await;
        assert!(res.is_ok());
        assert!(
            mempool
                .inner
                .read()
                .await
                .tx_intent_queue_pending_by_nonce
                .get(&**tx_new_3_mempool.contract_address())
                .expect("Missing nonce mapping for tx_new_3")
                .contains_key(&TransactionIntentPendingByNonce {
                    contract_address: **tx_new_3_mempool.contract_address(),
                    timestamp: tx_new_3_mempool.arrived_at,
                    nonce: tx_new_3_mempool.nonce,
                    nonce_next: tx_new_3_mempool.nonce_next,
                    phantom: std::marker::PhantomData
                }),
            "ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().await.tx_intent_queue_ready,
            mempool.inner.read().await.tx_intent_queue_pending_by_nonce
        );

        let arrived_at = TxTimestamp::UNIX_EPOCH;
        let tx_old_1_mempool = MempoolTransaction {
            tx: tx_old_1.into_blockifier().unwrap().0,
            arrived_at,
            converted_class: None,
            nonce: Nonce(Felt::ONE),
            nonce_next: Nonce(Felt::TWO),
        };
        let res = mempool
            .inner
            .insert_tx(tx_old_1_mempool.clone(), true, false, NonceInfo::ready(Nonce(Felt::ONE), Nonce(Felt::TWO)))
            .await;
        assert!(res.is_ok());
        assert!(
            mempool.inner.read().await.tx_intent_queue_ready.contains(&TransactionIntentReady {
                contract_address: **tx_old_1_mempool.contract_address(),
                timestamp: tx_old_1_mempool.arrived_at,
                nonce: tx_old_1_mempool.nonce,
                nonce_next: tx_old_1_mempool.nonce_next,
                phantom: std::marker::PhantomData
            }),
            "ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().await.tx_intent_queue_ready,
            mempool.inner.read().await.tx_intent_queue_pending_by_nonce
        );

        let arrived_at = TxTimestamp::UNIX_EPOCH;
        let tx_old_2_mempool = MempoolTransaction {
            tx: tx_old_2.into_blockifier().unwrap().0,
            arrived_at,
            converted_class: None,
            nonce: Nonce(Felt::ONE),
            nonce_next: Nonce(Felt::TWO),
        };
        let res = mempool
            .inner
            .insert_tx(tx_old_2_mempool.clone(), true, false, NonceInfo::ready(Nonce(Felt::ONE), Nonce(Felt::TWO)))
            .await;
        assert!(res.is_ok());
        assert!(
            mempool.inner.read().await.tx_intent_queue_ready.contains(&TransactionIntentReady {
                contract_address: **tx_old_2_mempool.contract_address(),
                timestamp: tx_old_2_mempool.arrived_at,
                nonce: tx_old_2_mempool.nonce,
                nonce_next: tx_old_2_mempool.nonce_next,
                phantom: std::marker::PhantomData
            }),
            "ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().await.tx_intent_queue_ready,
            mempool.inner.read().await.tx_intent_queue_pending_by_nonce
        );

        let arrived_at = TxTimestamp::UNIX_EPOCH;
        let tx_old_3_mempool = MempoolTransaction {
            tx: tx_old_3.into_blockifier().unwrap().0,
            arrived_at,
            converted_class: None,
            nonce: Nonce(Felt::TWO),
            nonce_next: Nonce(Felt::THREE),
        };
        let res = mempool
            .inner
            .insert_tx(tx_old_3_mempool.clone(), true, false, NonceInfo::pending(Nonce(Felt::TWO), Nonce(Felt::THREE)))
            .await;
        assert!(res.is_ok());
        assert!(
            mempool
                .inner
                .read()
                .await
                .tx_intent_queue_pending_by_nonce
                .get(&**tx_old_3_mempool.contract_address())
                .expect("Missing nonce mapping for tx_old_3")
                .contains_key(&TransactionIntentPendingByNonce {
                    contract_address: **tx_old_3_mempool.contract_address(),
                    timestamp: tx_old_3_mempool.arrived_at,
                    nonce: tx_old_3_mempool.nonce,
                    nonce_next: tx_old_3_mempool.nonce_next,
                    phantom: std::marker::PhantomData
                }),
            "ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().await.tx_intent_queue_ready,
            mempool.inner.read().await.tx_intent_queue_pending_by_nonce
        );

        let arrived_at = TxTimestamp::UNIX_EPOCH;
        let tx_old_4_mempool = MempoolTransaction {
            tx: tx_old_4.into_blockifier().unwrap().0,
            arrived_at,
            converted_class: None,
            nonce: Nonce(Felt::ONE),
            nonce_next: Nonce(Felt::TWO),
        };
        let res = mempool
            .inner
            .insert_tx(tx_old_4_mempool.clone(), true, false, NonceInfo::pending(Nonce(Felt::ONE), Nonce(Felt::TWO)))
            .await;
        assert!(res.is_ok());
        assert!(
            mempool
                .inner
                .read()
                .await
                .tx_intent_queue_pending_by_nonce
                .get(&**tx_old_4_mempool.contract_address())
                .expect("Missing nonce_mapping for tx_old_4")
                .contains_key(&TransactionIntentPendingByNonce {
                    contract_address: **tx_old_4_mempool.contract_address(),
                    timestamp: tx_old_4_mempool.arrived_at,
                    nonce: tx_old_4_mempool.nonce,
                    nonce_next: tx_old_4_mempool.nonce_next,
                    phantom: std::marker::PhantomData
                }),
            "ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().await.tx_intent_queue_ready,
            mempool.inner.read().await.tx_intent_queue_pending_by_nonce
        );

        // Make sure we have not entered an invalid state.
        mempool.inner.read().await.check_invariants();

        // ================================================================== //
        //                                STEP 2                              //
        // ================================================================== //

        // Now we actually remove the transactions. All the old transactions
        // should be removed.
        mempool.inner.write().await.remove_age_exceeded_txs();

        // tx_new_1 and tx_new_2 should still be in the mempool
        assert!(
            mempool.inner.read().await.tx_intent_queue_ready.contains(&TransactionIntentReady {
                contract_address: **tx_new_1_mempool.contract_address(),
                timestamp: tx_new_1_mempool.arrived_at,
                nonce: tx_new_1_mempool.nonce,
                nonce_next: tx_new_1_mempool.nonce_next,
                phantom: std::marker::PhantomData
            }),
            "ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().await.tx_intent_queue_ready,
            mempool.inner.read().await.tx_intent_queue_pending_by_nonce
        );
        assert!(
            mempool.inner.read().await.tx_intent_queue_ready.contains(&TransactionIntentReady {
                contract_address: **tx_new_2_mempool.contract_address(),
                timestamp: tx_new_2_mempool.arrived_at,
                nonce: tx_new_2_mempool.nonce,
                nonce_next: tx_new_2_mempool.nonce_next,
                phantom: std::marker::PhantomData
            }),
            "ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().await.tx_intent_queue_ready,
            mempool.inner.read().await.tx_intent_queue_pending_by_nonce
        );

        // tx_old_1 and tx_old_2 should no longer be in the ready queue
        assert!(
            !mempool.inner.read().await.tx_intent_queue_ready.contains(&TransactionIntentReady {
                contract_address: **tx_old_1_mempool.contract_address(),
                timestamp: tx_old_1_mempool.arrived_at,
                nonce: tx_old_1_mempool.nonce,
                nonce_next: tx_old_1_mempool.nonce_next,
                phantom: std::marker::PhantomData
            }),
            "ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().await.tx_intent_queue_ready,
            mempool.inner.read().await.tx_intent_queue_pending_by_nonce
        );
        assert!(
            !mempool.inner.read().await.tx_intent_queue_ready.contains(&TransactionIntentReady {
                contract_address: **tx_old_2_mempool.contract_address(),
                timestamp: tx_old_2_mempool.arrived_at,
                nonce: tx_old_2_mempool.nonce,
                nonce_next: tx_old_2_mempool.nonce_next,
                phantom: std::marker::PhantomData
            }),
            "ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().await.tx_intent_queue_ready,
            mempool.inner.read().await.tx_intent_queue_pending_by_nonce
        );

        // tx_new_3 should still be in the pending queue but tx_old_3 should not
        assert!(
            mempool
                .inner
                .read()
                .await
                .tx_intent_queue_pending_by_nonce
                .get(&**tx_new_3_mempool.contract_address())
                .expect("Missing nonce mapping for tx_new_3")
                .contains_key(&TransactionIntentPendingByNonce {
                    contract_address: **tx_new_3_mempool.contract_address(),
                    timestamp: tx_new_3_mempool.arrived_at,
                    nonce: tx_new_3_mempool.nonce,
                    nonce_next: tx_new_3_mempool.nonce_next,
                    phantom: std::marker::PhantomData
                }),
            "ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().await.tx_intent_queue_ready,
            mempool.inner.read().await.tx_intent_queue_pending_by_nonce
        );
        assert!(
            !mempool
                .inner
                .read()
                .await
                .tx_intent_queue_pending_by_nonce
                .get(&**tx_old_3_mempool.contract_address())
                .expect("Missing nonce mapping for tx_old_3")
                .contains_key(&TransactionIntentPendingByNonce {
                    contract_address: **tx_old_3_mempool.contract_address(),
                    timestamp: tx_old_3_mempool.arrived_at,
                    nonce: tx_old_3_mempool.nonce,
                    nonce_next: tx_old_3_mempool.nonce_next,
                    phantom: std::marker::PhantomData
                }),
            "ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().await.tx_intent_queue_ready,
            mempool.inner.read().await.tx_intent_queue_pending_by_nonce
        );

        // tx_old_4 should no longer be in the pending queue and since it was
        // the only transaction for that contract address, that pending queue
        // should have been emptied
        assert!(
            !mempool
                .inner
                .read()
                .await
                .tx_intent_queue_pending_by_nonce
                .contains_key(&**tx_old_4_mempool.contract_address()),
            "ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().await.tx_intent_queue_ready,
            mempool.inner.read().await.tx_intent_queue_pending_by_nonce
        );

        // Make sure we have not entered an invalid state.
        mempool.inner.read().await.check_invariants();
    }

    /// This test makes sure that adding an l1 handler to [MempoolInner] does
    /// not cause an infinite loop when trying to remove age exceeded
    /// transactions.
    ///
    /// This is important as age is not checked on l1 handlers, and so if they
    /// are not handled properly they cam cause an infinite loop.
    ///
    /// > This bug was originally detected through proptesting.
    #[rstest::rstest]
    #[timeout(Duration::from_millis(1_000))]
    #[tokio::test]
    async fn mempool_remove_aged_tx_pass_l1_handler(
        #[future] backend: Arc<mc_db::MadaraBackend>,
        tx_l1_handler_valid: ValidatedMempoolTx,
    ) {
        let backend = backend.await;
        let mempool = Mempool::new(backend, MempoolConfig::for_testing());

        let nonce_info = NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE));
        let mempool_tx = MempoolTransaction {
            tx: tx_l1_handler_valid.into_blockifier().unwrap().0,
            arrived_at: TxTimestamp::now(),
            converted_class: None,
            nonce: nonce_info.nonce,
            nonce_next: nonce_info.nonce_next,
        };

        let force = false;
        let update_limits = true;
        let result = mempool.inner.insert_tx(mempool_tx, force, update_limits, nonce_info).await;
        assert_matches::assert_matches!(result, Ok(()));

        let inner = mempool.inner.read().await;
        assert!(inner.nonce_is_ready(CONTRACT_ADDRESS, Nonce(Felt::ZERO)));
        inner.check_invariants();
        drop(inner);

        // This should not loop!
        mempool.inner.write().await.remove_age_exceeded_txs();
        let inner = mempool.inner.read().await;
        assert!(inner.nonce_is_ready(CONTRACT_ADDRESS, Nonce(Felt::ZERO)));
        inner.check_invariants();
    }

    /// This tests makes sure that if a transaction is inserted as [pending],
    /// and the transaction before it is polled, then that transaction becomes
    /// [ready].
    ///
    /// # Setup:
    ///
    /// - We assume `tx_ready` and `tx_pending` are from the same contract.
    ///
    /// - `tx_ready` has the correct nonce while `tx_pending` has the nonce
    ///   right after that.
    ///
    /// [ready]: inner::TransactionIntentReady;
    /// [pending]: inner::TransactionInentPending;
    #[tokio::test]
    #[rstest::rstest]
    async fn mempool_readiness_check(
        #[future] backend: Arc<mc_db::MadaraBackend>,
        #[from(tx_account_v0_valid)] mut tx_ready: ValidatedMempoolTx,
        #[from(tx_account_v0_valid)] mut tx_pending: ValidatedMempoolTx,
    ) {
        let backend = backend.await;
        let mempool = Mempool::new(backend, MempoolConfig::for_testing());

        // Insert pending transaction

        let nonce_info = NonceInfo::pending(Nonce(Felt::ONE), Nonce(Felt::TWO));
        let timestamp_pending = TxTimestamp::now();
        tx_pending.arrived_at = timestamp_pending;
        let result = mempool.accept_tx_with_nonce_info(tx_pending, nonce_info).await;
        assert_matches::assert_matches!(result, Ok(()));

        let inner = mempool.inner.read().await;
        inner.check_invariants();
        inner
            .tx_intent_queue_pending_by_nonce
            .get(&CONTRACT_ADDRESS)
            .expect("Mempool should have a pending queue for our contract address")
            .get(&TransactionIntentPendingByNonce {
                contract_address: CONTRACT_ADDRESS,
                timestamp: timestamp_pending,
                nonce: Nonce(Felt::ONE),
                nonce_next: Nonce(Felt::TWO),
                phantom: Default::default(),
            })
            .expect("Mempool should contain pending transaction");

        assert_eq!(inner.tx_intent_queue_pending_by_nonce.len(), 1);

        drop(inner);

        // Insert ready transaction

        let nonce_info = NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE));
        let timestamp_ready = TxTimestamp::now();
        tx_ready.arrived_at = timestamp_ready;
        let result = mempool.accept_tx_with_nonce_info(tx_ready, nonce_info).await;
        assert_matches::assert_matches!(result, Ok(()));

        let inner = mempool.inner.read().await;
        inner.check_invariants();
        inner
            .tx_intent_queue_ready
            .get(&TransactionIntentReady {
                contract_address: CONTRACT_ADDRESS,
                timestamp: timestamp_ready,
                nonce: Nonce(Felt::ZERO),
                nonce_next: Nonce(Felt::ONE),
                phantom: Default::default(),
            })
            .expect("Mempool should receive ready transaction");

        assert_eq!(inner.tx_intent_queue_ready.len(), 1);

        drop(inner);

        // Take the next transaction. The pending transaction was received first
        // but this should return the ready transaction instead.

        let mempool_tx = mempool.get_consumer().await.next().expect("Mempool contains a ready transaction!");

        let inner = mempool.inner.read().await;
        inner.check_invariants();
        inner
            .tx_intent_queue_ready
            .get(&TransactionIntentReady {
                contract_address: CONTRACT_ADDRESS,
                timestamp: timestamp_pending,
                nonce: Nonce(Felt::ONE),
                nonce_next: Nonce(Felt::TWO),
                phantom: Default::default(),
            })
            .expect("Mempool should have converted pending transaction to ready");

        // Taking the ready transaction should mark the pending transaction as
        // ready

        assert_eq!(mempool_tx.arrived_at, timestamp_ready);
        assert_eq!(inner.tx_intent_queue_ready.len(), 1);
        assert!(inner.tx_intent_queue_pending_by_nonce.is_empty());

        drop(inner);

        // Take the next transaction. The pending transaction has been marked as
        // ready and should be returned here

        let mempool_tx = mempool.get_consumer().await.next().expect("Mempool contains a ready transaction!");

        let inner = mempool.inner.read().await;
        inner.check_invariants();

        // No more transactions left

        assert_eq!(mempool_tx.arrived_at, timestamp_pending);
        assert!(inner.tx_intent_queue_ready.is_empty());
        assert!(inner.tx_intent_queue_pending_by_nonce.is_empty());
    }

    /// This tests makes sure that if a transaction is inserted into the
    /// [mempool], and its nonce does not match what is expected by the db BUT
    /// the transaction with the nonce preceding it is already marked as
    /// [ready], then it is marked as ready as well.
    ///
    /// This handles the case where the database has not yet been updated to
    /// reflect the state of the mempool, which should happen often since nonces
    /// are not updated until transaction execution. This way we limit the
    /// number of transactions we have in the [pending] queue.
    ///
    /// # Setup:
    ///
    /// - We assume `tx_1` and `tx_2` are from the same contract.
    ///
    /// - `tx_1` has the correct nonce while `tx_2` has the nonce right after
    ///   that.
    ///
    /// [mempool]: inner::MempoolInner
    /// [ready]: inner::TransactionIntentReady;
    /// [pending]: inner::TransactionInentPending;
    #[rstest::rstest]
    #[timeout(Duration::from_millis(1_000))]
    #[tokio::test]
    async fn mempool_readiness_check_against_db(
        #[future] backend: Arc<mc_db::MadaraBackend>,
        #[from(tx_account_v0_valid)] mut tx_1: ValidatedMempoolTx,
        #[from(tx_account_v0_valid)] mut tx_2: ValidatedMempoolTx,
    ) {
        let backend = backend.await;
        let mempool = Mempool::new(backend, MempoolConfig::for_testing());

        // Insert the first transaction

        let nonce_info =
            mempool.retrieve_nonce_info(CONTRACT_ADDRESS, Felt::ZERO).await.expect("Failed to retrieve nonce info");
        let timestamp_1 = TxTimestamp::now();
        tx_1.arrived_at = timestamp_1;

        let result = mempool.accept_tx_with_nonce_info(tx_1, nonce_info).await;
        assert_matches::assert_matches!(result, Ok(()));

        let inner = mempool.inner.read().await;
        inner.check_invariants();
        let contains = inner.tx_intent_queue_ready.contains(&TransactionIntentReady {
            contract_address: CONTRACT_ADDRESS,
            timestamp: timestamp_1,
            nonce: Nonce(Felt::ZERO),
            nonce_next: Nonce(Felt::ONE),
            phantom: Default::default(),
        });
        assert!(
            contains,
            "Mempool should contain transaction 1, ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().await.tx_intent_queue_ready,
            mempool.inner.read().await.tx_intent_queue_pending_by_nonce
        );

        assert_eq!(inner.tx_intent_queue_ready.len(), 1);

        drop(inner);

        // Insert the next transaction. It should be marked as ready even if it
        // does not have the correct nonce compared to the db (the test db
        // defaults to nonce 0 for all contracts) since the transaction before
        // it has already been marked as ready in the mempool.

        let nonce_info =
            mempool.retrieve_nonce_info(CONTRACT_ADDRESS, Felt::ONE).await.expect("Failed to retrieve nonce info");

        let timestamp_2 = TxTimestamp::now();
        tx_2.arrived_at = timestamp_2;
        let result = mempool.accept_tx_with_nonce_info(tx_2, nonce_info).await;
        assert_matches::assert_matches!(result, Ok(()));

        let inner = mempool.inner.read().await;
        inner.check_invariants();
        let contains = inner.tx_intent_queue_ready.contains(&TransactionIntentReady {
            contract_address: CONTRACT_ADDRESS,
            timestamp: timestamp_2,
            nonce: Nonce(Felt::ONE),
            nonce_next: Nonce(Felt::TWO),
            phantom: Default::default(),
        });
        assert!(
            contains,
            "Mempool should contain transaction 2, ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().await.tx_intent_queue_ready,
            mempool.inner.read().await.tx_intent_queue_pending_by_nonce
        );

        assert_eq!(inner.tx_intent_queue_ready.len(), 2);
    }

    /// This tests makes sure that a transaction is marked as [ready] if its
    /// [Nonce] is in the nonce cache for that contract address.
    ///
    /// This is used to check the readiness of transactions before committing
    /// the changes to db.
    ///
    /// # Setup:
    ///
    /// - We assume `tx_ready` and `tx_pending` are from the same contract.
    ///
    /// - `tx_ready` has the correct nonce while `tx_pending` has the nonce
    ///   right after that.
    ///
    /// [ready]: inner::TransactionIntentReady;
    #[rstest::rstest]
    #[timeout(Duration::from_millis(1_000))]
    #[tokio::test]
    async fn mempool_readiness_check_against_nonce_cache(#[future] backend: Arc<mc_db::MadaraBackend>) {
        let backend = backend.await;
        let mempool = Mempool::new(backend, MempoolConfig::for_testing());

        let contract_address = Felt::ZERO;
        let nonce = Nonce(Felt::ONE);
        let nonce_next = nonce.try_increment().unwrap();
        mempool.inner.nonce_cache_write().await.insert(contract_address, nonce);

        // Despite the tx nonce being Felt::ONE, it should be marked as ready
        // since it is present in the nonce cache
        assert_eq!(
            mempool.retrieve_nonce_info(contract_address, *nonce).await.unwrap(),
            NonceInfo::ready(nonce, nonce_next)
        );

        // We remove the nonce cache at that address
        let removed = mempool.inner.nonce_cache_write().await.remove(&contract_address);
        debug_assert!(removed.is_some());

        // Now the nonce should be marked as pending
        assert_eq!(
            mempool.retrieve_nonce_info(contract_address, *nonce).await.unwrap(),
            NonceInfo::pending(nonce, nonce_next)
        );
    }

    /// This test makes sure that retrieve nonce_info has proper error handling
    /// and returns correct values based on the state of the db.
    #[rstest::rstest]
    #[timeout(Duration::from_millis(1_000))]
    #[tokio::test]
    async fn mempool_retrieve_nonce_info(#[future] backend: Arc<mc_db::MadaraBackend>) {
        let backend = backend.await;
        let mempool = Mempool::new(Arc::clone(&backend), MempoolConfig::for_testing());

        backend
            .store_block(
                MadaraMaybePendingBlock {
                    info: MadaraMaybePendingBlockInfo::NotPending(MadaraBlockInfo::default()),
                    inner: MadaraBlockInner::default(),
                },
                StateDiff {
                    nonces: vec![NonceUpdate { contract_address: Felt::ZERO, nonce: Felt::ZERO }],
                    ..Default::default()
                },
                vec![],
            )
            .expect("Failed to store block");

        // Transactions which meet the nonce in db should be marked as ready...

        assert_eq!(
            mempool.retrieve_nonce_info(Felt::ZERO, Felt::ZERO).await.unwrap(),
            NonceInfo { readiness: NonceStatus::Ready, nonce: Nonce(Felt::ZERO), nonce_next: Nonce(Felt::ONE) }
        );

        // ...otherwise they should be marked as pending

        assert_eq!(
            mempool.retrieve_nonce_info(Felt::ZERO, Felt::ONE).await.unwrap(),
            NonceInfo { readiness: NonceStatus::Pending, nonce: Nonce(Felt::ONE), nonce_next: Nonce(Felt::TWO) }
        );

        // Contract addresses which do not yet have a nonce store in db should
        // should not fail during nonce info retrieval

        assert_eq!(
            mempool.retrieve_nonce_info(Felt::ONE, Felt::ZERO).await.unwrap(),
            NonceInfo { readiness: NonceStatus::Ready, nonce: Nonce(Felt::ZERO), nonce_next: Nonce(Felt::ONE) }
        );

        // Transactions with a nonce less than that in db should fail during
        // nonce info retrieval

        backend
            .store_block(
                MadaraMaybePendingBlock {
                    info: MadaraMaybePendingBlockInfo::NotPending(MadaraBlockInfo::default()),
                    inner: MadaraBlockInner::default(),
                },
                StateDiff {
                    nonces: vec![NonceUpdate { contract_address: Felt::ZERO, nonce: Felt::ONE }],
                    ..Default::default()
                },
                vec![],
            )
            .expect("Failed to store block");

        assert_matches::assert_matches!(
            mempool.retrieve_nonce_info(Felt::ZERO, Felt::ZERO).await,
            Err(MempoolError::InvalidNonce)
        );

        // We need to compute the next nonce inside retrieve nonce_info, so
        // passing Felt::MAX is not allowed.
        assert_matches::assert_matches!(
            mempool.retrieve_nonce_info(Felt::ZERO, Felt::MAX).await,
            Err(MempoolError::Internal(_))
        );
    }

    /// This test makes sure that resolve_nonce_info_l1_handler has proper
    /// error handling and returns correct values based on the state of the db.
    #[rstest::rstest]
    #[timeout(Duration::from_millis(1_000))]
    #[tokio::test]
    async fn mempool_resolve_nonce_info_l1_handler(#[future] backend: Arc<mc_db::MadaraBackend>) {
        let backend = backend.await;
        let mempool = Mempool::new(Arc::clone(&backend), MempoolConfig::for_testing());

        // First l1 handler tx (nonce == 0) is ready if there are nothing in db yet
        assert_eq!(
            mempool.resolve_nonce_info_l1_handler(Felt::ZERO).unwrap(),
            NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE))
        );

        // Non-zero l1 handler txs is accepted in pending state if older ones are still not processed
        assert_eq!(
            mempool.resolve_nonce_info_l1_handler(Felt::ONE).unwrap(),
            NonceInfo::pending(Nonce(Felt::ONE), Nonce(Felt::TWO))
        );

        // Updates the latest l1 nonce in db
        backend.set_l1_messaging_nonce(Nonce(Felt::ZERO)).expect("Failed to update l1 messaging nonce in db");

        // First l1 transaction has been stored in db. If we receive anything less than the next Nonce
        // We get an error

        assert_matches::assert_matches!(
            mempool.resolve_nonce_info_l1_handler(Felt::ZERO),
            Err(MempoolError::InvalidNonce)
        );

        // Following nonces should be marked as ready...

        assert_eq!(
            mempool.resolve_nonce_info_l1_handler(Felt::ONE).unwrap(),
            NonceInfo::ready(Nonce(Felt::ONE), Nonce(Felt::TWO))
        );

        // ...otherwise they should be marked as pending

        assert_eq!(
            mempool.resolve_nonce_info_l1_handler(Felt::TWO).unwrap(),
            NonceInfo::pending(Nonce(Felt::TWO), Nonce(Felt::THREE))
        );

        // We need to compute the next nonce inside retrieve nonce_info, so
        // passing Felt::MAX is not allowed.
        assert_matches::assert_matches!(
            mempool.resolve_nonce_info_l1_handler(Felt::MAX),
            Err(MempoolError::Internal(_))
        );
    }

    /// This test check the replacement logic for the [mempool] in case of force
    /// inserts.
    ///
    /// # Setup:
    ///
    /// - `tx_1` and `tx_2` are from the same account, `tx_3` is not.
    ///
    /// - `tx_1` and `tx_2` share the same nonce but a different timestamp.
    ///
    /// [mempool]: inner::MempoolInner
    #[rstest::rstest]
    #[timeout(Duration::from_millis(1_000))]
    #[tokio::test]
    async fn mempool_replace_pass(
        #[future] backend: Arc<mc_db::MadaraBackend>,
        #[from(tx_account_v0_valid)] tx_1: ValidatedMempoolTx,
        #[from(tx_account_v0_valid)] tx_2: ValidatedMempoolTx,
        #[from(tx_account_v0_valid)]
        #[with(Felt::ONE)]
        tx_3: ValidatedMempoolTx,
    ) {
        let backend = backend.await;
        let mempool = Mempool::new(backend, MempoolConfig::for_testing());

        // Insert the first transaction

        let force = true;
        let update_tx_limits = true;

        let arrived_at = TxTimestamp::now();
        let nonce_info = NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE));
        let tx_1_mempool = MempoolTransaction {
            tx: tx_1.into_blockifier().unwrap().0,
            arrived_at,
            converted_class: None,
            nonce: nonce_info.nonce,
            nonce_next: nonce_info.nonce_next,
        };
        let result = mempool.inner.insert_tx(tx_1_mempool.clone(), force, update_tx_limits, nonce_info).await;
        assert_matches::assert_matches!(result, Ok(()));

        let inner = mempool.inner.read().await;
        let contains = inner.tx_intent_queue_ready.contains(&TransactionIntentReady {
            contract_address: **tx_1_mempool.contract_address(),
            timestamp: tx_1_mempool.arrived_at,
            nonce: tx_1_mempool.nonce,
            nonce_next: tx_1_mempool.nonce_next,
            phantom: std::marker::PhantomData,
        });
        assert!(
            contains,
            "Mempool should contain transaction 1, ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().await.tx_intent_queue_ready,
            mempool.inner.read().await.tx_intent_queue_pending_by_nonce
        );

        assert_eq!(inner.tx_intent_queue_ready.len(), 1);

        inner.check_invariants();

        drop(inner);

        // Inserts the second transaction with the same nonce as the first once.
        // This should replace the first transaction.

        let force = true;
        let update_tx_limits = true;

        let arrived_at = arrived_at.checked_add(Duration::from_secs(1)).unwrap();
        let nonce_info = NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE));
        let tx_2_mempool = MempoolTransaction {
            tx: tx_2.into_blockifier().unwrap().0,
            arrived_at,
            converted_class: None,
            nonce: nonce_info.nonce,
            nonce_next: nonce_info.nonce_next,
        };
        let result = mempool.inner.insert_tx(tx_2_mempool.clone(), force, update_tx_limits, nonce_info).await;
        assert_matches::assert_matches!(result, Ok(()));

        let inner = mempool.inner.read().await;
        let contains = inner.tx_intent_queue_ready.contains(&TransactionIntentReady {
            contract_address: **tx_2_mempool.contract_address(),
            timestamp: tx_2_mempool.arrived_at,
            nonce: tx_2_mempool.nonce,
            nonce_next: tx_2_mempool.nonce_next,
            phantom: std::marker::PhantomData,
        });
        assert!(
            contains,
            "mempool should contain transaction 2, ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().await.tx_intent_queue_ready,
            mempool.inner.read().await.tx_intent_queue_pending_by_nonce
        );
        let contains = inner.tx_intent_queue_ready.contains(&TransactionIntentReady {
            contract_address: **tx_1_mempool.contract_address(),
            timestamp: tx_1_mempool.arrived_at,
            nonce: tx_1_mempool.nonce,
            nonce_next: tx_1_mempool.nonce_next,
            phantom: std::marker::PhantomData,
        });
        assert!(
            !contains,
            "Mempool should not contain transaction 1 after it has been replaced, ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().await.tx_intent_queue_ready,
            mempool.inner.read().await.tx_intent_queue_pending_by_nonce
        );

        assert_eq!(inner.tx_intent_queue_ready.len(), 1);

        inner.check_invariants();

        drop(inner);

        // Inserts the third transaction. This should not trigger a replacement
        // since it has a different contract address

        let force = true;
        let update_tx_limits = true;

        let arrived_at = arrived_at.checked_add(Duration::from_secs(1)).unwrap();
        let nonce_info = NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE));
        let tx_3_mempool = MempoolTransaction {
            tx: tx_3.into_blockifier().unwrap().0,
            arrived_at,
            converted_class: None,
            nonce: nonce_info.nonce,
            nonce_next: nonce_info.nonce_next,
        };
        let result = mempool.inner.insert_tx(tx_3_mempool.clone(), force, update_tx_limits, nonce_info.clone()).await;
        assert_matches::assert_matches!(result, Ok(()));

        let inner = mempool.inner.read().await;
        let contains = inner.tx_intent_queue_ready.contains(&TransactionIntentReady {
            contract_address: **tx_3_mempool.contract_address(),
            timestamp: tx_3_mempool.arrived_at,
            nonce: tx_3_mempool.nonce,
            nonce_next: tx_3_mempool.nonce_next,
            phantom: std::marker::PhantomData,
        });
        assert!(
            contains,
            "Mempool should contain transaction 3, ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().await.tx_intent_queue_ready,
            mempool.inner.read().await.tx_intent_queue_pending_by_nonce
        );
        let contains = inner.tx_intent_queue_ready.contains(&TransactionIntentReady {
            contract_address: **tx_2_mempool.contract_address(),
            timestamp: tx_2_mempool.arrived_at,
            nonce: tx_2_mempool.nonce,
            nonce_next: tx_2_mempool.nonce_next,
            phantom: std::marker::PhantomData,
        });
        assert!(
            contains,
            "mempool should contain transaction 2, ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().await.tx_intent_queue_ready,
            mempool.inner.read().await.tx_intent_queue_pending_by_nonce
        );

        assert_eq!(inner.tx_intent_queue_ready.len(), 2);

        inner.check_invariants();

        drop(inner);

        // Inserting a transaction again should fail if force if false. This
        // should not change the state of the mempool whatsoever!

        let force = false;
        let update_tx_limits = true;

        let result = mempool.inner.insert_tx(tx_3_mempool.clone(), force, update_tx_limits, nonce_info).await;
        assert_eq!(result, Err(TxInsertionError::DuplicateTxn));

        let inner = mempool.inner.read().await;
        let contains = inner.tx_intent_queue_ready.contains(&TransactionIntentReady {
            contract_address: **tx_3_mempool.contract_address(),
            timestamp: tx_3_mempool.arrived_at,
            nonce: tx_3_mempool.nonce,
            nonce_next: tx_3_mempool.nonce_next,
            phantom: std::marker::PhantomData,
        });
        assert!(
            contains,
            "Mempool should contain transaction 3, ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().await.tx_intent_queue_ready,
            mempool.inner.read().await.tx_intent_queue_pending_by_nonce
        );
        let contains = inner.tx_intent_queue_ready.contains(&TransactionIntentReady {
            contract_address: **tx_2_mempool.contract_address(),
            timestamp: tx_2_mempool.arrived_at,
            nonce: tx_2_mempool.nonce,
            nonce_next: tx_2_mempool.nonce_next,
            phantom: std::marker::PhantomData,
        });
        assert!(
            contains,
            "mempool should contain transaction 2, ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().await.tx_intent_queue_ready,
            mempool.inner.read().await.tx_intent_queue_pending_by_nonce
        );

        assert_eq!(inner.tx_intent_queue_ready.len(), 2);

        inner.check_invariants();
    }
}
