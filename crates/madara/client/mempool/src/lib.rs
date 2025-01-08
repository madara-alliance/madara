use anyhow::Context;
use blockifier::blockifier::stateful_validator::StatefulValidatorError;
use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::transaction_execution::Transaction;
use blockifier::transaction::transactions::{
    DeclareTransaction, DeployAccountTransaction, InvokeTransaction, L1HandlerTransaction as BL1HandlerTransaction,
};
use header::make_pending_header;
use mc_db::db_block_id::DbBlockId;
use mc_db::mempool_db::{DbMempoolTxInfoDecoder, NonceInfo};
use mc_db::{MadaraBackend, MadaraStorageError};
use mc_exec::ExecutionContext;
use metrics::MempoolMetrics;
use mp_block::{BlockId, BlockTag, MadaraPendingBlockInfo};
use mp_class::ConvertedClass;
use mp_convert::ToFelt;
use mp_transactions::BroadcastedDeclareTransactionV0;
use mp_transactions::BroadcastedTransactionExt;
use mp_transactions::L1HandlerTransaction;
use mp_transactions::L1HandlerTransactionResult;
use mp_transactions::ToBlockifierError;
use starknet_api::core::{ContractAddress, Nonce};
use starknet_api::transaction::TransactionHash;
use starknet_api::StarknetApiError;
use starknet_types_core::felt::Felt;
use starknet_types_rpc::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn,
    BroadcastedTxn, ClassAndTxnHash, ContractAndTxnHash,
};
use std::sync::{Arc, RwLock};
use std::time::SystemTime;
use tx::blockifier_to_saved_tx;
use tx::saved_to_blockifier_tx;

#[cfg(any(test, feature = "testing"))]
pub use l1::MockL1DataProvider;
pub use l1::{GasPriceProvider, L1DataProvider};

pub mod header;
mod inner;
mod l1;
pub mod metrics;
mod tx;

pub use inner::*;

#[derive(thiserror::Error, Debug)]
pub enum MempoolError {
    #[error("Storage error: {0:#}")]
    StorageError(#[from] MadaraStorageError),
    #[error("Validation error: {0:#}")]
    Validation(#[from] StatefulValidatorError),
    #[error(transparent)]
    InnerMempool(#[from] TxInsertionError),
    #[error(transparent)]
    Exec(#[from] mc_exec::Error),
    #[error(transparent)]
    StarknetApi(#[from] StarknetApiError),
    #[error("Preprocessing transaction: {0:#}")]
    BroadcastedToBlockifier(#[from] ToBlockifierError),
}
impl MempoolError {
    pub fn is_internal(&self) -> bool {
        matches!(self, MempoolError::StorageError(_) | MempoolError::BroadcastedToBlockifier(_))
    }
}

#[cfg(test)]
pub(crate) trait CheckInvariants {
    /// Validates the invariants of this struct.
    ///
    /// This method should cause a panic if these invariants are not met.
    fn check_invariants(&self);
}

#[cfg_attr(test, mockall::automock)]
pub trait MempoolProvider: Send + Sync {
    fn accept_invoke_tx(
        &self,
        tx: BroadcastedInvokeTxn<Felt>,
    ) -> Result<AddInvokeTransactionResult<Felt>, MempoolError>;
    fn accept_declare_v0_tx(&self, tx: BroadcastedDeclareTransactionV0) -> Result<ClassAndTxnHash<Felt>, MempoolError>;
    fn accept_declare_tx(&self, tx: BroadcastedDeclareTxn<Felt>) -> Result<ClassAndTxnHash<Felt>, MempoolError>;
    fn accept_deploy_account_tx(
        &self,
        tx: BroadcastedDeployAccountTxn<Felt>,
    ) -> Result<ContractAndTxnHash<Felt>, MempoolError>;
    fn accept_l1_handler_tx(
        &self,
        tx: L1HandlerTransaction,
        paid_fees_on_l1: u128,
    ) -> Result<L1HandlerTransactionResult, MempoolError>;
    fn take_txs_chunk<I: Extend<MempoolTransaction> + 'static>(&self, dest: &mut I, n: usize)
    where
        Self: Sized;
    fn take_tx(&self) -> Option<MempoolTransaction>;
    fn re_add_txs<I: IntoIterator<Item = MempoolTransaction> + 'static>(
        &self,
        txs: I,
        consumed_txs: Vec<MempoolTransaction>,
    ) -> Result<(), MempoolError>
    where
        Self: Sized;
    fn insert_txs_no_validation(&self, txs: Vec<MempoolTransaction>, force: bool) -> Result<(), MempoolError>
    where
        Self: Sized;
    fn chain_id(&self) -> Felt;
}

pub struct Mempool {
    backend: Arc<MadaraBackend>,
    l1_data_provider: Arc<dyn L1DataProvider>,
    inner: RwLock<MempoolInner>,
    metrics: MempoolMetrics,
}

impl Mempool {
    pub fn new(backend: Arc<MadaraBackend>, l1_data_provider: Arc<dyn L1DataProvider>, limits: MempoolLimits) -> Self {
        Mempool {
            backend,
            l1_data_provider,
            inner: RwLock::new(MempoolInner::new(limits)),
            metrics: MempoolMetrics::register(),
        }
    }

    pub fn load_txs_from_db(&mut self) -> Result<(), anyhow::Error> {
        for res in self.backend.get_mempool_transactions() {
            let (tx_hash, DbMempoolTxInfoDecoder { saved_tx, converted_class, nonce_readiness }) =
                res.context("Getting mempool transactions")?;
            let (tx, arrived_at) = saved_to_blockifier_tx(saved_tx, tx_hash, &converted_class)
                .context("Converting saved tx to blockifier")?;

            if let Err(err) = self.accept_tx(tx, converted_class, arrived_at, nonce_readiness) {
                match err {
                    MempoolError::InnerMempool(TxInsertionError::Limit(MempoolLimitReached::Age { .. })) => {} // do nothing
                    err => tracing::warn!("Could not re-add mempool transaction from db: {err:#}"),
                }
            }
        }
        Ok(())
    }

    #[tracing::instrument(skip(self), fields(module = "Mempool"))]
    fn accept_tx(
        &self,
        tx: Transaction,
        converted_class: Option<ConvertedClass>,
        arrived_at: SystemTime,
        nonce_info: NonceInfo,
    ) -> Result<(), MempoolError> {
        // Get pending block.
        let pending_block_info = if let Some(block) = self.backend.get_block_info(&DbBlockId::Pending)? {
            block
        } else {
            // No current pending block, we'll make an unsaved empty one for the sake of validating this tx.
            let parent_block_hash = self
                .backend
                .get_block_hash(&BlockId::Tag(BlockTag::Latest))?
                .unwrap_or(/* genesis block's parent hash */ Felt::ZERO);
            MadaraPendingBlockInfo::new(
                make_pending_header(parent_block_hash, self.backend.chain_config(), self.l1_data_provider.as_ref()),
                vec![],
            )
            .into()
        };

        // If the contract has been deployed for the same block is is invoked, we need to skip validations.
        // NB: the lock is NOT taken the entire time the tx is being validated. As such, the deploy tx
        //  may appear during that time - but it is not a problem.
        let deploy_account_tx_hash = if let Transaction::AccountTransaction(AccountTransaction::Invoke(tx)) = &tx {
            let mempool = self.inner.read().expect("Poisoned lock");
            if mempool.has_deployed_contract(&tx.tx.sender_address()) {
                Some(tx.tx_hash) // we return the wrong tx hash here but it's ok because the actual hash is unused by blockifier
            } else {
                None
            }
        } else {
            None
        };

        let tx_hash = tx_hash(&tx).to_felt();
        tracing::debug!("Mempool verify tx_hash={:#x}", tx_hash);

        // Perform validations
        let exec_context = ExecutionContext::new_at_block_end(Arc::clone(&self.backend), &pending_block_info)?;
        let mut validator = exec_context.tx_validator();

        if let Transaction::AccountTransaction(account_tx) = clone_transaction(&tx) {
            validator.perform_validations(account_tx, deploy_account_tx_hash.is_some())?
        }

        if !is_only_query(&tx) {
            tracing::debug!("Adding to inner mempool tx_hash={:#x}", tx_hash);
            // Add to db
            let saved_tx = blockifier_to_saved_tx(&tx, arrived_at);

            self.backend.save_mempool_transaction(&saved_tx, tx_hash, &converted_class, &nonce_info)?;

            // Add it to the inner mempool
            let force = false;
            let nonce = nonce_info.nonce;
            let nonce_next = nonce_info.nonce_next;
            self.inner.write().expect("Poisoned lock").insert_tx(
                MempoolTransaction { tx, arrived_at, converted_class, nonce, nonce_next },
                force,
                true,
                nonce_info,
            )?;

            self.metrics.accepted_transaction_counter.add(1, &[]);
        }

        Ok(())
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn is_empty(&self) -> bool {
        self.inner.read().expect("Poisoned lock").is_empty()
    }

    fn retrieve_nonce_info(&self, sender_address: Felt, nonce: Felt) -> Result<NonceInfo, MempoolError> {
        let nonce = Nonce(nonce);
        let nonce_next = nonce.try_increment()?;
        let nonce_target = self
            .backend
            .get_contract_nonce_at(&BlockId::Tag(BlockTag::Latest), &sender_address)?
            .map(|nonce| Nonce(nonce).try_increment())
            .unwrap_or(Ok(nonce))?;

        if nonce != nonce_target {
            // We don't need an underflow check as the default value for
            // nonce_target in db is 0. Since nonce != nonce_target, we already
            // know that nonce != 0.
            let nonce_prev = Nonce(nonce.0 - Felt::ONE);
            let nonce_prev_ready = self.inner.read().expect("Poisoned lock").nonce_is_ready(sender_address, nonce_prev);

            // If the mempool has the transaction before this one ready, then
            // this transaction is ready too. Even if the db has not been
            // updated yet, this transaction will always be polled and executed
            // afterwards.
            if nonce_prev_ready {
                Ok(NonceInfo::ready(nonce, nonce_next))
            } else {
                // BUG: what if a transaction is received AFTER the previous tx
                // has been polled from the mempool, but BEFORE the db has been
                // updated? It will never be moved to ready!
                Ok(NonceInfo::pending(nonce, nonce_next))
            }
        } else {
            Ok(NonceInfo::ready(nonce, nonce_next))
        }
    }
}

pub fn transaction_hash(tx: &Transaction) -> Felt {
    match tx {
        Transaction::AccountTransaction(tx) => match tx {
            AccountTransaction::Declare(tx) => *tx.tx_hash,
            AccountTransaction::DeployAccount(tx) => *tx.tx_hash,
            AccountTransaction::Invoke(tx) => *tx.tx_hash,
        },
        Transaction::L1HandlerTransaction(tx) => *tx.tx_hash,
    }
}

fn declare_class_hash(tx: &Transaction) -> Option<Felt> {
    match tx {
        Transaction::AccountTransaction(AccountTransaction::Declare(tx)) => Some(*tx.class_hash()),
        _ => None,
    }
}

fn deployed_contract_address(tx: &Transaction) -> Option<Felt> {
    match tx {
        Transaction::AccountTransaction(AccountTransaction::DeployAccount(tx)) => Some(**tx.contract_address),
        _ => None,
    }
}

impl MempoolProvider for Mempool {
    #[tracing::instrument(skip(self), fields(module = "Mempool"))]
    fn accept_invoke_tx(
        &self,
        tx: BroadcastedInvokeTxn<Felt>,
    ) -> Result<AddInvokeTransactionResult<Felt>, MempoolError> {
        let nonce_info = match &tx {
            BroadcastedInvokeTxn::V1(ref tx) => self.retrieve_nonce_info(tx.sender_address, tx.nonce)?,
            BroadcastedInvokeTxn::V3(ref tx) => self.retrieve_nonce_info(tx.sender_address, tx.nonce)?,
            BroadcastedInvokeTxn::QueryV1(ref tx) => self.retrieve_nonce_info(tx.sender_address, tx.nonce)?,
            BroadcastedInvokeTxn::QueryV3(ref tx) => self.retrieve_nonce_info(tx.sender_address, tx.nonce)?,
            BroadcastedInvokeTxn::V0(_) | &BroadcastedInvokeTxn::QueryV0(_) => NonceInfo::default(),
        };

        let tx = BroadcastedTxn::Invoke(tx);
        let (btx, class) = tx.into_blockifier(self.chain_id(), self.backend.chain_config().latest_protocol_version)?;

        let res = AddInvokeTransactionResult { transaction_hash: transaction_hash(&btx) };
        self.accept_tx(btx, class, ArrivedAtTimestamp::now(), nonce_info)?;
        Ok(res)
    }

    #[tracing::instrument(skip(self), fields(module = "Mempool"))]
    fn accept_declare_v0_tx(&self, tx: BroadcastedDeclareTransactionV0) -> Result<ClassAndTxnHash<Felt>, MempoolError> {
        let (btx, class) = tx.into_blockifier(self.chain_id(), self.backend.chain_config().latest_protocol_version)?;

        let res = ClassAndTxnHash {
            transaction_hash: transaction_hash(&btx),
            class_hash: declare_class_hash(&btx).expect("Created transaction should be declare"),
        };

        self.accept_tx(btx, class, ArrivedAtTimestamp::now(), NonceInfo::default())?;
        Ok(res)
    }

    #[tracing::instrument(skip(self), fields(module = "Mempool"))]
    fn accept_l1_handler_tx(
        &self,
        tx: L1HandlerTransaction,
        paid_fees_on_l1: u128,
    ) -> Result<L1HandlerTransactionResult, MempoolError> {
        let nonce = Nonce(Felt::from(tx.nonce));
        let (btx, class) =
            tx.into_blockifier(self.chain_id(), self.backend.chain_config().latest_protocol_version, paid_fees_on_l1)?;

        // L1 Handler nonces represent the ordering of L1 transactions sent by
        // the core L1 contract. In principle this is a bit strange, as there
        // currently is only 1 core L1 contract, so all transactions should be
        // ordered by default. Moreover, these transaction are infrequent, so
        // the risk that two transactions are emitted at very short intervals
        // seems unlikely. Still, who knows?
        //
        // INFO: L1 nonce are stored differently in the db because of this, which is
        // why we do not use `retrieve_nonce_readiness`.
        let nonce_next = nonce.try_increment()?;
        let nonce_target =
            self.backend.get_l1_messaging_nonce_latest()?.map(|nonce| nonce.try_increment()).unwrap_or(Ok(nonce))?;
        let nonce_info = if nonce != nonce_target {
            NonceInfo::pending(nonce, nonce_next)
        } else {
            NonceInfo::ready(nonce, nonce_next)
        };

        let res = L1HandlerTransactionResult { transaction_hash: transaction_hash(&btx) };
        self.accept_tx(btx, class, ArrivedAtTimestamp::now(), nonce_info)?;
        Ok(res)
    }

    #[tracing::instrument(skip(self), fields(module = "Mempool"))]
    fn accept_declare_tx(&self, tx: BroadcastedDeclareTxn<Felt>) -> Result<ClassAndTxnHash<Felt>, MempoolError> {
        let nonce_info = match &tx {
            BroadcastedDeclareTxn::V1(ref tx) => self.retrieve_nonce_info(tx.sender_address, tx.nonce)?,
            BroadcastedDeclareTxn::V2(ref tx) => self.retrieve_nonce_info(tx.sender_address, tx.nonce)?,
            BroadcastedDeclareTxn::V3(ref tx) => self.retrieve_nonce_info(tx.sender_address, tx.nonce)?,
            BroadcastedDeclareTxn::QueryV1(ref tx) => self.retrieve_nonce_info(tx.sender_address, tx.nonce)?,
            BroadcastedDeclareTxn::QueryV2(ref tx) => self.retrieve_nonce_info(tx.sender_address, tx.nonce)?,
            BroadcastedDeclareTxn::QueryV3(ref tx) => self.retrieve_nonce_info(tx.sender_address, tx.nonce)?,
        };

        let tx = BroadcastedTxn::Declare(tx);
        let (btx, class) = tx.into_blockifier(self.chain_id(), self.backend.chain_config().latest_protocol_version)?;

        let res = ClassAndTxnHash {
            transaction_hash: transaction_hash(&btx),
            class_hash: declare_class_hash(&btx).expect("Created transaction should be declare"),
        };
        self.accept_tx(btx, class, ArrivedAtTimestamp::now(), nonce_info)?;
        Ok(res)
    }

    #[tracing::instrument(skip(self), fields(module = "Mempool"))]
    fn accept_deploy_account_tx(
        &self,
        tx: BroadcastedDeployAccountTxn<Felt>,
    ) -> Result<ContractAndTxnHash<Felt>, MempoolError> {
        let tx = BroadcastedTxn::DeployAccount(tx);
        let (btx, class) = tx.into_blockifier(self.chain_id(), self.backend.chain_config().latest_protocol_version)?;

        let res = ContractAndTxnHash {
            transaction_hash: transaction_hash(&btx),
            contract_address: deployed_contract_address(&btx).expect("Created transaction should be deploy account"),
        };
        self.accept_tx(btx, class, ArrivedAtTimestamp::now(), NonceInfo::default())?;
        Ok(res)
    }

    /// Warning: A lock is held while a user-supplied function (extend) is run - Callers should be careful
    #[tracing::instrument(skip(self, dest, n), fields(module = "Mempool"))]
    fn take_txs_chunk<I: Extend<MempoolTransaction> + 'static>(&self, dest: &mut I, n: usize) {
        let mut inner = self.inner.write().expect("Poisoned lock");
        inner.pop_next_chunk(dest, n)
    }

    #[tracing::instrument(skip(self), fields(module = "Mempool"))]
    fn take_tx(&self) -> Option<MempoolTransaction> {
        let mut inner = self.inner.write().expect("Poisoned lock");
        inner.pop_next()
    }

    /// Warning: A lock is taken while a user-supplied function (iterator stuff)
    /// is run - Callers should be careful This is called by the block
    /// production after a batch of transaction is executed. Mark the consumed
    /// txs as consumed, and re-add the transactions that are not consumed in
    /// the mempool.
    #[tracing::instrument(skip(self, txs, consumed_txs), fields(module = "Mempool"))]
    fn re_add_txs<I: IntoIterator<Item = MempoolTransaction>>(
        &self,
        txs: I,
        consumed_txs: Vec<MempoolTransaction>,
    ) -> Result<(), MempoolError> {
        let mut inner = self.inner.write().expect("Poisoned lock");
        let hashes = consumed_txs.iter().map(|tx| tx.tx_hash()).collect::<Vec<_>>();
        inner.re_add_txs(txs, consumed_txs);
        drop(inner);
        for tx_hash in hashes {
            self.backend.remove_mempool_transaction(&tx_hash.to_felt())?;
        }
        Ok(())
    }

    /// This is called by the block production task to re-add transaction from
    /// the pending block back into the mempool
    #[tracing::instrument(skip(self, txs), fields(module = "Mempool"))]
    fn insert_txs_no_validation(&self, txs: Vec<MempoolTransaction>, force: bool) -> Result<(), MempoolError> {
        for tx in &txs {
            let saved_tx = blockifier_to_saved_tx(&tx.tx, tx.arrived_at);
            // Save to db. Transactions are marked as ready since they were
            // already previously included into the pending block
            let nonce_info = NonceInfo::ready(tx.nonce, tx.nonce_next);
            self.backend.save_mempool_transaction(
                &saved_tx,
                tx.tx_hash().to_felt(),
                &tx.converted_class,
                &nonce_info,
            )?;
        }
        let mut inner = self.inner.write().expect("Poisoned lock");
        inner.insert_txs(txs, force)?;

        Ok(())
    }
    fn chain_id(&self) -> Felt {
        Felt::from_bytes_be_slice(format!("{}", self.backend.chain_config().chain_id).as_bytes())
    }
}

pub(crate) fn is_only_query(tx: &Transaction) -> bool {
    match tx {
        Transaction::AccountTransaction(account_tx) => match account_tx {
            AccountTransaction::Declare(tx) => tx.only_query(),
            AccountTransaction::DeployAccount(tx) => tx.only_query,
            AccountTransaction::Invoke(tx) => tx.only_query,
        },
        Transaction::L1HandlerTransaction(_) => false,
    }
}

pub(crate) fn contract_addr(tx: &Transaction) -> ContractAddress {
    match tx {
        Transaction::AccountTransaction(account_tx) => match account_tx {
            AccountTransaction::Declare(tx) => tx.tx.sender_address(),
            AccountTransaction::DeployAccount(tx) => tx.contract_address,
            AccountTransaction::Invoke(tx) => tx.tx.sender_address(),
        },
        Transaction::L1HandlerTransaction(tx) => tx.tx.contract_address,
    }
}

pub(crate) fn nonce(tx: &Transaction) -> Nonce {
    match tx {
        Transaction::AccountTransaction(account_tx) => match account_tx {
            AccountTransaction::Declare(tx) => tx.tx.nonce(),
            AccountTransaction::DeployAccount(tx) => tx.tx.nonce(),
            AccountTransaction::Invoke(tx) => tx.tx.nonce(),
        },
        Transaction::L1HandlerTransaction(tx) => tx.tx.nonce,
    }
}

pub(crate) fn tx_hash(tx: &Transaction) -> TransactionHash {
    match tx {
        Transaction::AccountTransaction(account_tx) => match account_tx {
            AccountTransaction::Declare(tx) => tx.tx_hash,
            AccountTransaction::DeployAccount(tx) => tx.tx_hash,
            AccountTransaction::Invoke(tx) => tx.tx_hash,
        },
        Transaction::L1HandlerTransaction(tx) => tx.tx_hash,
    }
}

// AccountTransaction does not implement Clone :(
pub(crate) fn clone_transaction(tx: &Transaction) -> Transaction {
    match tx {
        Transaction::AccountTransaction(account_tx) => Transaction::AccountTransaction(match account_tx {
            AccountTransaction::Declare(tx) => AccountTransaction::Declare(match tx.only_query() {
                true => DeclareTransaction::new_for_query(tx.tx.clone(), tx.tx_hash, tx.class_info.clone())
                    .expect("Making blockifier transaction for query"),
                false => DeclareTransaction::new(tx.tx.clone(), tx.tx_hash, tx.class_info.clone())
                    .expect("Making blockifier transaction"),
            }),
            AccountTransaction::DeployAccount(tx) => AccountTransaction::DeployAccount(DeployAccountTransaction {
                tx: tx.tx.clone(),
                tx_hash: tx.tx_hash,
                contract_address: tx.contract_address,
                only_query: tx.only_query,
            }),
            AccountTransaction::Invoke(tx) => AccountTransaction::Invoke(InvokeTransaction {
                tx: tx.tx.clone(),
                tx_hash: tx.tx_hash,
                only_query: tx.only_query,
            }),
        }),
        Transaction::L1HandlerTransaction(tx) => Transaction::L1HandlerTransaction(BL1HandlerTransaction {
            tx: tx.tx.clone(),
            tx_hash: tx.tx_hash,
            paid_fee_on_l1: tx.paid_fee_on_l1,
        }),
    }
}

#[cfg(test)]
mod test {
    use std::time::Duration;

    use super::*;

    #[rstest::fixture]
    fn backend() -> Arc<mc_db::MadaraBackend> {
        mc_db::MadaraBackend::open_for_testing(Arc::new(mp_chain_config::ChainConfig::madara_test()))
    }

    #[rstest::fixture]
    fn l1_data_provider() -> Arc<MockL1DataProvider> {
        let mut mock = MockL1DataProvider::new();
        mock.expect_get_gas_prices().return_const(mp_block::header::GasPrices {
            eth_l1_gas_price: 0,
            strk_l1_gas_price: 0,
            eth_l1_data_gas_price: 0,
            strk_l1_data_gas_price: 0,
        });
        mock.expect_get_gas_prices_last_update().return_const(std::time::SystemTime::now());
        mock.expect_get_da_mode().return_const(mp_block::header::L1DataAvailabilityMode::Calldata);
        Arc::new(mock)
    }

    #[rstest::fixture]
    fn tx_account_v0_valid(
        #[default(Felt::ZERO)] contract_address: Felt,
    ) -> blockifier::transaction::transaction_execution::Transaction {
        blockifier::transaction::transaction_execution::Transaction::AccountTransaction(
            blockifier::transaction::account_transaction::AccountTransaction::Invoke(
                blockifier::transaction::transactions::InvokeTransaction {
                    tx: starknet_api::transaction::InvokeTransaction::V0(
                        starknet_api::transaction::InvokeTransactionV0 {
                            contract_address: ContractAddress::try_from(contract_address).unwrap(),
                            ..Default::default()
                        },
                    ),
                    tx_hash: starknet_api::transaction::TransactionHash::default(),
                    only_query: false,
                },
            ),
        )
    }

    #[rstest::fixture]
    fn tx_account_v1_invalid() -> blockifier::transaction::transaction_execution::Transaction {
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
        )
    }

    #[rstest::rstest]
    fn mempool_accept_tx_pass(
        backend: Arc<mc_db::MadaraBackend>,
        l1_data_provider: Arc<MockL1DataProvider>,
        tx_account_v0_valid: blockifier::transaction::transaction_execution::Transaction,
    ) {
        let mempool = Mempool::new(backend, l1_data_provider, MempoolLimits::for_testing());
        let result = mempool.accept_tx(tx_account_v0_valid, None, ArrivedAtTimestamp::now(), NonceInfo::default());
        assert_matches::assert_matches!(result, Ok(()));

        mempool.inner.read().expect("Poisoned lock").check_invariants();
    }

    #[rstest::rstest]
    fn mempool_accept_tx_fail_validate(
        backend: Arc<mc_db::MadaraBackend>,
        l1_data_provider: Arc<MockL1DataProvider>,
        tx_account_v1_invalid: blockifier::transaction::transaction_execution::Transaction,
    ) {
        let mempool = Mempool::new(backend, l1_data_provider, MempoolLimits::for_testing());
        let result = mempool.accept_tx(tx_account_v1_invalid, None, ArrivedAtTimestamp::now(), NonceInfo::default());
        assert_matches::assert_matches!(result, Err(crate::MempoolError::Validation(_)));

        mempool.inner.read().expect("Poisoned lock").check_invariants();
    }

    #[rstest::rstest]
    fn mempool_take_tx_pass(
        backend: Arc<mc_db::MadaraBackend>,
        l1_data_provider: Arc<MockL1DataProvider>,
        tx_account_v0_valid: blockifier::transaction::transaction_execution::Transaction,
    ) {
        let mempool = Mempool::new(backend, l1_data_provider, MempoolLimits::for_testing());
        let timestamp = ArrivedAtTimestamp::now();
        let result = mempool.accept_tx(tx_account_v0_valid, None, timestamp, NonceInfo::default());
        assert_matches::assert_matches!(result, Ok(()));

        let mempool_tx = mempool.take_tx().expect("Mempool should contain a transaction");
        assert_eq!(mempool_tx.arrived_at, timestamp);

        mempool.inner.read().expect("Poisoned lock").check_invariants();
    }

    /// This test makes sure that old transactions are removed from the mempool,
    /// whether they be represented by ready or pending intents.
    ///
    /// # Setup
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
    #[rstest::rstest]
    fn mempool_remove_aged_tx_pass(
        backend: Arc<mc_db::MadaraBackend>,
        l1_data_provider: Arc<MockL1DataProvider>,
        #[from(tx_account_v0_valid)] // We are reusing the tx_account_v0_valid fixture...
        #[with(Felt::ZERO)] // ...with different arguments
        tx_new_1: blockifier::transaction::transaction_execution::Transaction,
        #[from(tx_account_v0_valid)]
        #[with(Felt::ONE)]
        tx_new_2: blockifier::transaction::transaction_execution::Transaction,
        #[from(tx_account_v0_valid)]
        #[with(Felt::ONE)]
        tx_new_3: blockifier::transaction::transaction_execution::Transaction,
        #[from(tx_account_v0_valid)]
        #[with(Felt::ZERO)]
        tx_old_1: blockifier::transaction::transaction_execution::Transaction,
        #[from(tx_account_v0_valid)]
        #[with(Felt::ONE)]
        tx_old_2: blockifier::transaction::transaction_execution::Transaction,
        #[from(tx_account_v0_valid)]
        #[with(Felt::TWO)]
        tx_old_3: blockifier::transaction::transaction_execution::Transaction,
        #[from(tx_account_v0_valid)]
        #[with(Felt::THREE)]
        tx_old_4: blockifier::transaction::transaction_execution::Transaction,
    ) {
        let mempool = Mempool::new(
            backend,
            l1_data_provider,
            MempoolLimits { max_age: Some(Duration::from_secs(3600)), ..MempoolLimits::for_testing() },
        );

        // ================================================================== //
        //                                STEP 1                              //
        // ================================================================== //

        // First, we begin by inserting all our transactions and making sure
        // they are in the ready as well as the pending intent queues.
        let arrived_at = ArrivedAtTimestamp::now();
        let tx_new_1_mempool = MempoolTransaction {
            tx: tx_new_1,
            arrived_at,
            converted_class: None,
            nonce: Nonce(Felt::ZERO),
            nonce_next: Nonce(Felt::ONE),
        };
        let res = mempool.inner.write().expect("Poisoned lock").insert_tx(
            tx_new_1_mempool.clone(),
            true,
            false,
            NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE)),
        );
        assert!(res.is_ok());
        assert!(
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready.contains(&TransactionIntentReady {
                contract_address: **tx_new_1_mempool.contract_address(),
                timestamp: tx_new_1_mempool.arrived_at,
                nonce: tx_new_1_mempool.nonce,
                nonce_next: tx_new_1_mempool.nonce_next,
                phantom: std::marker::PhantomData
            }),
            "ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready,
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending
        );

        let arrived_at = ArrivedAtTimestamp::now();
        let tx_new_2_mempool = MempoolTransaction {
            tx: tx_new_2,
            arrived_at,
            converted_class: None,
            nonce: Nonce(Felt::ZERO),
            nonce_next: Nonce(Felt::ONE),
        };
        let res = mempool.inner.write().expect("Poisoned lock").insert_tx(
            tx_new_2_mempool.clone(),
            true,
            false,
            NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE)),
        );
        assert!(res.is_ok());
        assert!(
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready.contains(&TransactionIntentReady {
                contract_address: **tx_new_2_mempool.contract_address(),
                timestamp: tx_new_2_mempool.arrived_at,
                nonce: tx_new_2_mempool.nonce,
                nonce_next: tx_new_2_mempool.nonce_next,
                phantom: std::marker::PhantomData
            }),
            "ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready,
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending
        );

        let arrived_at = ArrivedAtTimestamp::now();
        let tx_new_3_mempool = MempoolTransaction {
            tx: tx_new_3,
            arrived_at,
            converted_class: None,
            nonce: Nonce(Felt::ONE),
            nonce_next: Nonce(Felt::TWO),
        };
        let res = mempool.inner.write().expect("Poisoned lock").insert_tx(
            tx_new_3_mempool.clone(),
            true,
            false,
            NonceInfo::pending(Nonce(Felt::ONE), Nonce(Felt::TWO)),
        );
        assert!(res.is_ok());
        assert!(
            mempool
                .inner
                .read()
                .expect("Poisoned lock")
                .tx_intent_queue_pending
                .get(&**tx_new_3_mempool.contract_address())
                .expect("Missing nonce mapping for tx_new_3")
                .contains(&TransactionIntentPending {
                    contract_address: **tx_new_3_mempool.contract_address(),
                    timestamp: tx_new_3_mempool.arrived_at,
                    nonce: tx_new_3_mempool.nonce,
                    nonce_next: tx_new_3_mempool.nonce_next,
                    phantom: std::marker::PhantomData
                }),
            "ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready,
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending
        );

        let arrived_at = ArrivedAtTimestamp::UNIX_EPOCH;
        let tx_old_1_mempool = MempoolTransaction {
            tx: tx_old_1,
            arrived_at,
            converted_class: None,
            nonce: Nonce(Felt::ONE),
            nonce_next: Nonce(Felt::TWO),
        };
        let res = mempool.inner.write().expect("Poisoned lock").insert_tx(
            tx_old_1_mempool.clone(),
            true,
            false,
            NonceInfo::ready(Nonce(Felt::ONE), Nonce(Felt::TWO)),
        );
        assert!(res.is_ok());
        assert!(
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready.contains(&TransactionIntentReady {
                contract_address: **tx_old_1_mempool.contract_address(),
                timestamp: tx_old_1_mempool.arrived_at,
                nonce: tx_old_1_mempool.nonce,
                nonce_next: tx_old_1_mempool.nonce_next,
                phantom: std::marker::PhantomData
            }),
            "ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready,
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending
        );

        let arrived_at = ArrivedAtTimestamp::UNIX_EPOCH;
        let tx_old_2_mempool = MempoolTransaction {
            tx: tx_old_2,
            arrived_at,
            converted_class: None,
            nonce: Nonce(Felt::ONE),
            nonce_next: Nonce(Felt::TWO),
        };
        let res = mempool.inner.write().expect("Poisoned lock").insert_tx(
            tx_old_2_mempool.clone(),
            true,
            false,
            NonceInfo::ready(Nonce(Felt::ONE), Nonce(Felt::TWO)),
        );
        assert!(res.is_ok());
        assert!(
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready.contains(&TransactionIntentReady {
                contract_address: **tx_old_2_mempool.contract_address(),
                timestamp: tx_old_2_mempool.arrived_at,
                nonce: tx_old_2_mempool.nonce,
                nonce_next: tx_old_2_mempool.nonce_next,
                phantom: std::marker::PhantomData
            }),
            "ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready,
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending
        );

        let arrived_at = ArrivedAtTimestamp::UNIX_EPOCH;
        let tx_old_3_mempool = MempoolTransaction {
            tx: tx_old_3,
            arrived_at,
            converted_class: None,
            nonce: Nonce(Felt::TWO),
            nonce_next: Nonce(Felt::THREE),
        };
        let res = mempool.inner.write().expect("Poisoned lock").insert_tx(
            tx_old_3_mempool.clone(),
            true,
            false,
            NonceInfo::pending(Nonce(Felt::TWO), Nonce(Felt::THREE)),
        );
        assert!(res.is_ok());
        assert!(
            mempool
                .inner
                .read()
                .expect("Poisoned lock")
                .tx_intent_queue_pending
                .get(&**tx_old_3_mempool.contract_address())
                .expect("Missing nonce mapping for tx_old_3")
                .contains(&TransactionIntentPending {
                    contract_address: **tx_old_3_mempool.contract_address(),
                    timestamp: tx_old_3_mempool.arrived_at,
                    nonce: tx_old_3_mempool.nonce,
                    nonce_next: tx_old_3_mempool.nonce_next,
                    phantom: std::marker::PhantomData
                }),
            "ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready,
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending
        );

        let arrived_at = ArrivedAtTimestamp::UNIX_EPOCH;
        let tx_old_4_mempool = MempoolTransaction {
            tx: tx_old_4,
            arrived_at,
            converted_class: None,
            nonce: Nonce(Felt::ONE),
            nonce_next: Nonce(Felt::TWO),
        };
        let res = mempool.inner.write().expect("Poisoned lock").insert_tx(
            tx_old_4_mempool.clone(),
            true,
            false,
            NonceInfo::pending(Nonce(Felt::ONE), Nonce(Felt::TWO)),
        );
        assert!(res.is_ok());
        assert!(
            mempool
                .inner
                .read()
                .expect("Poisoned lock")
                .tx_intent_queue_pending
                .get(&**tx_old_4_mempool.contract_address())
                .expect("Missing nonce_mapping for tx_old_4")
                .contains(&TransactionIntentPending {
                    contract_address: **tx_old_4_mempool.contract_address(),
                    timestamp: tx_old_4_mempool.arrived_at,
                    nonce: tx_old_4_mempool.nonce,
                    nonce_next: tx_old_4_mempool.nonce_next,
                    phantom: std::marker::PhantomData
                }),
            "ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready,
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending
        );

        // Make sure we have not entered an invalid state.
        mempool.inner.read().expect("Poisoned lock").check_invariants();

        // ================================================================== //
        //                                STEP 2                              //
        // ================================================================== //

        // Now we actually remove the transactions. All the old transactions
        // should be removed.
        mempool.inner.write().expect("Poisoned lock").remove_age_exceeded_txs();

        // tx_new_1 and tx_new_2 should still be in the mempool
        assert!(
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready.contains(&TransactionIntentReady {
                contract_address: **tx_new_1_mempool.contract_address(),
                timestamp: tx_new_1_mempool.arrived_at,
                nonce: tx_new_1_mempool.nonce,
                nonce_next: tx_new_1_mempool.nonce_next,
                phantom: std::marker::PhantomData
            }),
            "ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready,
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending
        );
        assert!(
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready.contains(&TransactionIntentReady {
                contract_address: **tx_new_2_mempool.contract_address(),
                timestamp: tx_new_2_mempool.arrived_at,
                nonce: tx_new_2_mempool.nonce,
                nonce_next: tx_new_2_mempool.nonce_next,
                phantom: std::marker::PhantomData
            }),
            "ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready,
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending
        );

        // tx_old_1 and tx_old_2 should no longer be in the ready queue
        assert!(
            !mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready.contains(&TransactionIntentReady {
                contract_address: **tx_old_1_mempool.contract_address(),
                timestamp: tx_old_1_mempool.arrived_at,
                nonce: tx_old_1_mempool.nonce,
                nonce_next: tx_old_1_mempool.nonce_next,
                phantom: std::marker::PhantomData
            }),
            "ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready,
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending
        );
        assert!(
            !mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready.contains(&TransactionIntentReady {
                contract_address: **tx_old_2_mempool.contract_address(),
                timestamp: tx_old_2_mempool.arrived_at,
                nonce: tx_old_2_mempool.nonce,
                nonce_next: tx_old_2_mempool.nonce_next,
                phantom: std::marker::PhantomData
            }),
            "ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready,
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending
        );

        // tx_new_3 should still be in the pending queue but tx_old_3 should not
        assert!(
            mempool
                .inner
                .read()
                .expect("Poisoned lock")
                .tx_intent_queue_pending
                .get(&**tx_new_3_mempool.contract_address())
                .expect("Missing nonce mapping for tx_new_3")
                .contains(&TransactionIntentPending {
                    contract_address: **tx_new_3_mempool.contract_address(),
                    timestamp: tx_new_3_mempool.arrived_at,
                    nonce: tx_new_3_mempool.nonce,
                    nonce_next: tx_new_3_mempool.nonce_next,
                    phantom: std::marker::PhantomData
                }),
            "ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready,
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending
        );
        assert!(
            !mempool
                .inner
                .read()
                .expect("Poisoned lock")
                .tx_intent_queue_pending
                .get(&**tx_old_3_mempool.contract_address())
                .expect("Missing nonce mapping for tx_old_3")
                .contains(&TransactionIntentPending {
                    contract_address: **tx_old_3_mempool.contract_address(),
                    timestamp: tx_old_3_mempool.arrived_at,
                    nonce: tx_old_3_mempool.nonce,
                    nonce_next: tx_old_3_mempool.nonce_next,
                    phantom: std::marker::PhantomData
                }),
            "ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready,
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending
        );

        // tx_old_4 should no longer be in the pending queue and since it was
        // the only transaction for that contract address, that pending queue
        // should have been emptied
        assert!(
            mempool
                .inner
                .read()
                .expect("Poisoned lock")
                .tx_intent_queue_pending
                .get(&**tx_old_4_mempool.contract_address())
                .is_none(),
            "ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready,
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending
        );

        // Make sure we have not entered an invalid state.
        mempool.inner.read().expect("Poisoned lock").check_invariants();
    }

    #[rstest::rstest]
    fn mempool_readiness_check(
        backend: Arc<mc_db::MadaraBackend>,
        l1_data_provider: Arc<MockL1DataProvider>,
        #[from(tx_account_v0_valid)] tx_ready: blockifier::transaction::transaction_execution::Transaction,
        #[from(tx_account_v0_valid)] tx_pending: blockifier::transaction::transaction_execution::Transaction,
    ) {
        let mempool = Mempool::new(backend, l1_data_provider, MempoolLimits::for_testing());

        // Insert pending transaction

        let nonce_info = NonceInfo::pending(Nonce(Felt::ONE), Nonce(Felt::TWO));
        let timestamp_pending = ArrivedAtTimestamp::now();
        let result = mempool.accept_tx(tx_pending, None, timestamp_pending, nonce_info);
        assert_matches::assert_matches!(result, Ok(()));

        let inner = mempool.inner.read().expect("Poisoned lock");
        inner.check_invariants();
        inner
            .tx_intent_queue_pending
            .get(&Felt::ZERO)
            .expect("Mempool should have a pending queue for contract address ZERO")
            .get(&TransactionIntentPending {
                contract_address: Felt::ZERO,
                timestamp: timestamp_pending,
                nonce: Nonce(Felt::ONE),
                nonce_next: Nonce(Felt::TWO),
                phantom: Default::default(),
            })
            .expect("Mempool should contain pending transaction");

        assert_eq!(inner.tx_intent_queue_pending.len(), 1);

        drop(inner);

        // Insert ready transaction

        let nonce_info = NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE));
        let timestamp_ready = ArrivedAtTimestamp::now();
        let result = mempool.accept_tx(tx_ready, None, timestamp_ready, nonce_info);
        assert_matches::assert_matches!(result, Ok(()));

        let inner = mempool.inner.read().expect("Poisoned lock");
        inner.check_invariants();
        inner
            .tx_intent_queue_ready
            .get(&TransactionIntentReady {
                contract_address: Felt::ZERO,
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

        let mempool_tx = mempool.take_tx().expect("Mempool contains a ready transaction!");

        let inner = mempool.inner.read().expect("Poisoned lock");
        inner.check_invariants();
        inner
            .tx_intent_queue_ready
            .get(&TransactionIntentReady {
                contract_address: Felt::ZERO,
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
        assert!(inner.tx_intent_queue_pending.is_empty());

        drop(inner);

        // Take the next transaction. The pending transaction has been marked as
        // ready and should be returned here

        let mempool_tx = mempool.take_tx().expect("Mempool contains a ready transaction!");

        let inner = mempool.inner.read().expect("Poisoned lock");
        inner.check_invariants();

        // No more transactions left

        assert_eq!(mempool_tx.arrived_at, timestamp_pending);
        assert!(inner.tx_intent_queue_ready.is_empty());
        assert!(inner.tx_intent_queue_pending.is_empty());
    }
}
