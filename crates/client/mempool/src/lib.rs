use anyhow::Context;
use blockifier::blockifier::stateful_validator::StatefulValidatorError;
use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::transaction_execution::Transaction;
use blockifier::transaction::transactions::{
    DeclareTransaction, DeployAccountTransaction, InvokeTransaction, L1HandlerTransaction as BL1HandlerTransaction,
};
use header::make_pending_header;
use mc_db::db_block_id::DbBlockId;
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
pub enum Error {
    #[error("Storage error: {0:#}")]
    StorageError(#[from] MadaraStorageError),
    #[error("Validation error: {0:#}")]
    Validation(#[from] StatefulValidatorError),
    #[error(transparent)]
    InnerMempool(#[from] TxInsersionError),
    #[error(transparent)]
    Exec(#[from] mc_exec::Error),
    #[error("Preprocessing transaction: {0:#}")]
    BroadcastedToBlockifier(#[from] ToBlockifierError),
}
impl Error {
    pub fn is_internal(&self) -> bool {
        matches!(self, Error::StorageError(_) | Error::BroadcastedToBlockifier(_))
    }
}

#[cfg_attr(test, mockall::automock)]
pub trait MempoolProvider: Send + Sync {
    fn accept_invoke_tx(&self, tx: BroadcastedInvokeTxn<Felt>) -> Result<AddInvokeTransactionResult<Felt>, Error>;
    fn accept_declare_v0_tx(&self, tx: BroadcastedDeclareTransactionV0) -> Result<ClassAndTxnHash<Felt>, Error>;
    fn accept_declare_tx(&self, tx: BroadcastedDeclareTxn<Felt>) -> Result<ClassAndTxnHash<Felt>, Error>;
    fn accept_deploy_account_tx(
        &self,
        tx: BroadcastedDeployAccountTxn<Felt>,
    ) -> Result<ContractAndTxnHash<Felt>, Error>;
    fn accept_l1_handler_tx(
        &self,
        tx: L1HandlerTransaction,
        paid_fees_on_l1: u128,
    ) -> Result<L1HandlerTransactionResult, Error>;
    fn take_txs_chunk<I: Extend<MempoolTransaction> + 'static>(&self, dest: &mut I, n: usize)
    where
        Self: Sized;
    fn take_tx(&self) -> Option<MempoolTransaction>;
    fn re_add_txs<I: IntoIterator<Item = MempoolTransaction> + 'static>(
        &self,
        txs: I,
        consumed_txs: Vec<MempoolTransaction>,
    ) -> Result<(), Error>
    where
        Self: Sized;
    fn insert_txs_no_validation(&self, txs: Vec<MempoolTransaction>, force: bool) -> Result<(), Error>
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
            let (tx_hash, saved_tx, converted_class) = res.context("Getting mempool transactions")?;
            let (tx, arrived_at) = saved_to_blockifier_tx(saved_tx, tx_hash, &converted_class)
                .context("Converting saved tx to blockifier")?;

            if let Err(err) = self.accept_tx(tx, converted_class, arrived_at) {
                match err {
                    Error::InnerMempool(TxInsersionError::Limit(MempoolLimitReached::Age { .. })) => {} // do nothing
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
    ) -> Result<(), Error> {
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
            self.backend.save_mempool_transaction(&saved_tx, tx_hash, &converted_class)?;

            // Add it to the inner mempool
            let force = false;
            self.inner.write().expect("Poisoned lock").insert_tx(
                MempoolTransaction { tx, arrived_at, converted_class },
                force,
                true,
            )?;

            self.metrics.accepted_transaction_counter.add(1, &[]);
        }

        Ok(())
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn is_empty(&self) -> bool {
        self.inner.read().expect("Poisoned lock").is_empty()
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
    fn accept_invoke_tx(&self, tx: BroadcastedInvokeTxn<Felt>) -> Result<AddInvokeTransactionResult<Felt>, Error> {
        let tx = BroadcastedTxn::Invoke(tx);
        let (btx, class) = tx.into_blockifier(self.chain_id(), self.backend.chain_config().latest_protocol_version)?;

        let res = AddInvokeTransactionResult { transaction_hash: transaction_hash(&btx) };
        self.accept_tx(btx, class, ArrivedAtTimestamp::now())?;
        Ok(res)
    }

    #[tracing::instrument(skip(self), fields(module = "Mempool"))]
    fn accept_declare_v0_tx(&self, tx: BroadcastedDeclareTransactionV0) -> Result<ClassAndTxnHash<Felt>, Error> {
        let (btx, class) = tx.into_blockifier(self.chain_id(), self.backend.chain_config().latest_protocol_version)?;

        let res = ClassAndTxnHash {
            transaction_hash: transaction_hash(&btx),
            class_hash: declare_class_hash(&btx).expect("Created transaction should be declare"),
        };
        self.accept_tx(btx, class, ArrivedAtTimestamp::now())?;
        Ok(res)
    }

    #[tracing::instrument(skip(self), fields(module = "Mempool"))]
    fn accept_l1_handler_tx(
        &self,
        tx: L1HandlerTransaction,
        paid_fees_on_l1: u128,
    ) -> Result<L1HandlerTransactionResult, Error> {
        let (btx, class) =
            tx.into_blockifier(self.chain_id(), self.backend.chain_config().latest_protocol_version, paid_fees_on_l1)?;

        let res = L1HandlerTransactionResult { transaction_hash: transaction_hash(&btx) };
        self.accept_tx(btx, class, ArrivedAtTimestamp::now())?;
        Ok(res)
    }

    #[tracing::instrument(skip(self), fields(module = "Mempool"))]
    fn accept_declare_tx(&self, tx: BroadcastedDeclareTxn<Felt>) -> Result<ClassAndTxnHash<Felt>, Error> {
        let tx = BroadcastedTxn::Declare(tx);
        let (btx, class) = tx.into_blockifier(self.chain_id(), self.backend.chain_config().latest_protocol_version)?;

        let res = ClassAndTxnHash {
            transaction_hash: transaction_hash(&btx),
            class_hash: declare_class_hash(&btx).expect("Created transaction should be declare"),
        };
        self.accept_tx(btx, class, ArrivedAtTimestamp::now())?;
        Ok(res)
    }

    #[tracing::instrument(skip(self), fields(module = "Mempool"))]
    fn accept_deploy_account_tx(
        &self,
        tx: BroadcastedDeployAccountTxn<Felt>,
    ) -> Result<ContractAndTxnHash<Felt>, Error> {
        let tx = BroadcastedTxn::DeployAccount(tx);
        let (btx, class) = tx.into_blockifier(self.chain_id(), self.backend.chain_config().latest_protocol_version)?;

        let res = ContractAndTxnHash {
            transaction_hash: transaction_hash(&btx),
            contract_address: deployed_contract_address(&btx).expect("Created transaction should be deploy account"),
        };
        self.accept_tx(btx, class, ArrivedAtTimestamp::now())?;
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

    /// Warning: A lock is taken while a user-supplied function (iterator stuff) is run - Callers should be careful
    /// This is called by the block production after a batch of transaction is executed.
    /// Mark the consumed txs as consumed, and re-add the transactions that are not consumed in the mempool.
    #[tracing::instrument(skip(self, txs, consumed_txs), fields(module = "Mempool"))]
    fn re_add_txs<I: IntoIterator<Item = MempoolTransaction>>(
        &self,
        txs: I,
        consumed_txs: Vec<MempoolTransaction>,
    ) -> Result<(), Error> {
        let mut inner = self.inner.write().expect("Poisoned lock");
        let hashes = consumed_txs.iter().map(|tx| tx.tx_hash()).collect::<Vec<_>>();
        inner.re_add_txs(txs, consumed_txs);
        drop(inner);
        for tx_hash in hashes {
            self.backend.remove_mempool_transaction(&tx_hash.to_felt())?;
        }
        Ok(())
    }

    #[tracing::instrument(skip(self, txs), fields(module = "Mempool"))]
    fn insert_txs_no_validation(&self, txs: Vec<MempoolTransaction>, force: bool) -> Result<(), Error> {
        for tx in &txs {
            let saved_tx = blockifier_to_saved_tx(&tx.tx, tx.arrived_at);
            // save to db
            self.backend.save_mempool_transaction(&saved_tx, tx.tx_hash().to_felt(), &tx.converted_class)?;
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
    fn tx_account_v0_valid() -> blockifier::transaction::transaction_execution::Transaction {
        blockifier::transaction::transaction_execution::Transaction::AccountTransaction(
            blockifier::transaction::account_transaction::AccountTransaction::Invoke(
                blockifier::transaction::transactions::InvokeTransaction {
                    tx: starknet_api::transaction::InvokeTransaction::V0(
                        starknet_api::transaction::InvokeTransactionV0::default(),
                    ),
                    tx_hash: starknet_api::transaction::TransactionHash(Felt::default()),
                    only_query: true,
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
                    tx_hash: starknet_api::transaction::TransactionHash(Felt::default()),
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
        let result = mempool.accept_tx(tx_account_v0_valid, None, ArrivedAtTimestamp::now());
        assert_matches::assert_matches!(result, Ok(()));
    }

    #[rstest::rstest]
    fn mempool_accept_tx_fail_validate(
        backend: Arc<mc_db::MadaraBackend>,
        l1_data_provider: Arc<MockL1DataProvider>,
        tx_account_v1_invalid: blockifier::transaction::transaction_execution::Transaction,
    ) {
        let mempool = Mempool::new(backend, l1_data_provider, MempoolLimits::for_testing());
        let result = mempool.accept_tx(tx_account_v1_invalid, None, ArrivedAtTimestamp::now());
        assert_matches::assert_matches!(result, Err(crate::Error::Validation(_)));
    }
}
