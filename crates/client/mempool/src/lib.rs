use blockifier::blockifier::stateful_validator::StatefulValidatorError;
use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::transaction_execution::Transaction;
use blockifier::transaction::transactions::{
    DeclareTransaction, DeployAccountTransaction, InvokeTransaction, L1HandlerTransaction,
};
use header::make_pending_header;
use inner::MempoolInner;
use mc_db::db_block_id::DbBlockId;
use mc_db::{MadaraBackend, MadaraStorageError};
use mc_exec::ExecutionContext;
use metrics::MempoolMetrics;
use mp_block::{BlockId, BlockTag, MadaraPendingBlockInfo};
use mp_class::ConvertedClass;
use mp_convert::ToFelt;
use mp_transactions::{
    broadcasted_declare_v0_to_blockifier, broadcasted_to_blockifier, BroadcastedDeclareTransactionV0,
};
use mp_transactions::{BroadcastedToBlockifierError, L1HandlerTransactionResult};
use starknet_api::core::{ContractAddress, Nonce};
use starknet_api::transaction::TransactionHash;
use starknet_types_core::felt::Felt;
use starknet_types_rpc::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn,
    BroadcastedTxn, ClassAndTxnHash, ContractAndTxnHash,
};
use std::sync::{Arc, RwLock};

pub use inner::TxInsersionError;
pub use inner::{ArrivedAtTimestamp, MempoolTransaction};
#[cfg(any(test, feature = "testing"))]
pub use l1::MockL1DataProvider;
pub use l1::{GasPriceProvider, L1DataProvider};

pub mod block_production;
pub mod block_production_metrics;
mod close_block;
pub mod header;
mod inner;
mod l1;
pub mod metrics;

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
    BroadcastedToBlockifier(#[from] BroadcastedToBlockifierError),
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
    fn accept_l1_handler_tx(&self, tx: Transaction) -> Result<L1HandlerTransactionResult, Error>;
    fn take_txs_chunk<I: Extend<MempoolTransaction> + 'static>(&self, dest: &mut I, n: usize)
    where
        Self: Sized;
    fn take_tx(&self) -> Option<MempoolTransaction>;
    fn re_add_txs<I: IntoIterator<Item = MempoolTransaction> + 'static>(&self, txs: I)
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
    pub fn new(backend: Arc<MadaraBackend>, l1_data_provider: Arc<dyn L1DataProvider>) -> Self {
        Mempool { backend, l1_data_provider, inner: Default::default(), metrics: MempoolMetrics::register() }
    }

    #[tracing::instrument(skip(self), fields(module = "Mempool"))]
    fn accept_tx(&self, tx: Transaction, converted_class: Option<ConvertedClass>) -> Result<(), Error> {
        // The timestamp *does not* take the transaction validation time into account.
        let arrived_at = ArrivedAtTimestamp::now();

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

        tracing::debug!("Mempool verify tx_hash={:#x}", tx_hash(&tx).to_felt());

        // Perform validations
        let exec_context = ExecutionContext::new_in_block(Arc::clone(&self.backend), &pending_block_info)?;
        let mut validator = exec_context.tx_validator();

        if let Transaction::AccountTransaction(account_tx) = clone_transaction(&tx) {
            validator.perform_validations(account_tx, deploy_account_tx_hash.is_some())?
        }

        if !is_only_query(&tx) {
            tracing::debug!("Adding to mempool tx_hash={:#x}", tx_hash(&tx).to_felt());
            // Finally, add it to the nonce chain for the account nonce
            let force = false;
            self.inner
                .write()
                .expect("Poisoned lock")
                .insert_tx(MempoolTransaction { tx, arrived_at, converted_class }, force)?
        }

        self.metrics.accepted_transaction_counter.add(1, &[]);

        Ok(())
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
        let (tx, classes) = broadcasted_to_blockifier(
            BroadcastedTxn::Invoke(tx),
            self.chain_id(),
            self.backend.chain_config().latest_protocol_version,
        )?;

        let res = AddInvokeTransactionResult { transaction_hash: transaction_hash(&tx) };
        self.accept_tx(tx, classes)?;
        Ok(res)
    }

    #[tracing::instrument(skip(self), fields(module = "Mempool"))]
    fn accept_declare_v0_tx(&self, tx: BroadcastedDeclareTransactionV0) -> Result<ClassAndTxnHash<Felt>, Error> {
        let (tx, classes) = broadcasted_declare_v0_to_blockifier(
            tx,
            self.chain_id(),
            self.backend.chain_config().latest_protocol_version,
        )?;

        let res = ClassAndTxnHash {
            transaction_hash: transaction_hash(&tx),
            class_hash: declare_class_hash(&tx).expect("Created transaction should be declare"),
        };

        self.accept_tx(tx, classes)?;
        Ok(res)
    }

    fn accept_l1_handler_tx(&self, tx: Transaction) -> Result<L1HandlerTransactionResult, Error> {
        let res = L1HandlerTransactionResult { transaction_hash: transaction_hash(&tx) };
        self.accept_tx(tx, None)?;
        Ok(res)
    }

    #[tracing::instrument(skip(self), fields(module = "Mempool"))]
    fn accept_declare_tx(&self, tx: BroadcastedDeclareTxn<Felt>) -> Result<ClassAndTxnHash<Felt>, Error> {
        let (tx, classes) = broadcasted_to_blockifier(
            BroadcastedTxn::Declare(tx),
            self.chain_id(),
            self.backend.chain_config().latest_protocol_version,
        )?;

        let res = ClassAndTxnHash {
            transaction_hash: transaction_hash(&tx),
            class_hash: declare_class_hash(&tx).expect("Created transaction should be declare"),
        };
        self.accept_tx(tx, classes)?;
        Ok(res)
    }

    #[tracing::instrument(skip(self), fields(module = "Mempool"))]
    fn accept_deploy_account_tx(
        &self,
        tx: BroadcastedDeployAccountTxn<Felt>,
    ) -> Result<ContractAndTxnHash<Felt>, Error> {
        let (tx, classes) = broadcasted_to_blockifier(
            BroadcastedTxn::DeployAccount(tx),
            self.chain_id(),
            self.backend.chain_config().latest_protocol_version,
        )?;

        let res = ContractAndTxnHash {
            transaction_hash: transaction_hash(&tx),
            contract_address: deployed_contract_address(&tx).expect("Created transaction should be deploy account"),
        };
        self.accept_tx(tx, classes)?;
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
    #[tracing::instrument(skip(self, txs), fields(module = "Mempool"))]
    fn re_add_txs<I: IntoIterator<Item = MempoolTransaction> + 'static>(&self, txs: I) {
        let mut inner = self.inner.write().expect("Poisoned lock");
        inner.re_add_txs(txs)
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
        Transaction::L1HandlerTransaction(tx) => Transaction::L1HandlerTransaction(L1HandlerTransaction {
            tx: tx.tx.clone(),
            tx_hash: tx.tx_hash,
            paid_fee_on_l1: tx.paid_fee_on_l1,
        }),
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use starknet_types_core::felt::Felt;

    use crate::MockL1DataProvider;

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
        let mempool = crate::Mempool::new(backend, l1_data_provider);
        let result = mempool.accept_tx(tx_account_v0_valid, None);
        assert_matches::assert_matches!(result, Ok(()));
    }

    #[rstest::rstest]
    fn mempool_accept_tx_fail_validate(
        backend: Arc<mc_db::MadaraBackend>,
        l1_data_provider: Arc<MockL1DataProvider>,
        tx_account_v1_invalid: blockifier::transaction::transaction_execution::Transaction,
    ) {
        let mempool = crate::Mempool::new(backend, l1_data_provider);
        let result = mempool.accept_tx(tx_account_v1_invalid, None);
        assert_matches::assert_matches!(result, Err(crate::Error::Validation(_)));
    }
}
