use blockifier::blockifier::stateful_validator::StatefulValidatorError;
use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::transaction_execution::Transaction;
use blockifier::transaction::transactions::DeclareTransaction;
use blockifier::transaction::transactions::DeployAccountTransaction;
use blockifier::transaction::transactions::InvokeTransaction;
use header::make_pending_header;
use inner::MempoolInner;
use mc_db::db_block_id::DbBlockId;
use mc_db::MadaraBackend;
use mc_db::MadaraStorageError;
use mc_exec::ExecutionContext;
use mp_block::BlockId;
use mp_block::BlockTag;
use mp_block::MadaraPendingBlockInfo;
use mp_class::ConvertedClass;
use mp_transactions::broadcasted_to_blockifier;
use mp_transactions::BroadcastedToBlockifierError;
use starknet_api::core::{ContractAddress, Nonce};
use starknet_api::transaction::TransactionHash;
use starknet_core::types::BroadcastedDeclareTransaction;
use starknet_core::types::BroadcastedDeployAccountTransaction;
use starknet_core::types::BroadcastedInvokeTransaction;
use starknet_core::types::BroadcastedTransaction;
use starknet_core::types::DeclareTransactionResult;
use starknet_core::types::DeployAccountTransactionResult;
use starknet_core::types::InvokeTransactionResult;
use starknet_types_core::felt::Felt;
use std::sync::Arc;
use std::sync::RwLock;

pub use inner::TxInsersionError;
pub use inner::{ArrivedAtTimestamp, MempoolTransaction};
#[cfg(any(test, feature = "testing"))]
pub use l1::MockL1DataProvider;
pub use l1::{GasPriceProvider, L1DataProvider};

pub mod block_production;
mod close_block;
pub mod header;
mod inner;
mod l1;

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
    fn accept_invoke_tx(&self, tx: BroadcastedInvokeTransaction) -> Result<InvokeTransactionResult, Error>;
    fn accept_declare_tx(&self, tx: BroadcastedDeclareTransaction) -> Result<DeclareTransactionResult, Error>;
    fn accept_deploy_account_tx(
        &self,
        tx: BroadcastedDeployAccountTransaction,
    ) -> Result<DeployAccountTransactionResult, Error>;
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
}

impl Mempool {
    pub fn new(backend: Arc<MadaraBackend>, l1_data_provider: Arc<dyn L1DataProvider>) -> Self {
        Mempool { backend, l1_data_provider, inner: Default::default() }
    }

    fn accept_tx(&self, tx: Transaction, converted_class: Option<ConvertedClass>) -> Result<(), Error> {
        let Transaction::AccountTransaction(tx) = tx else { panic!("L1HandlerTransaction not supported yet") };

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
        let deploy_account_tx_hash = if let AccountTransaction::Invoke(tx) = &tx {
            let mempool = self.inner.read().expect("Poisoned lock");
            if mempool.has_deployed_contract(&tx.tx.sender_address()) {
                Some(tx.tx_hash) // we return the wrong tx hash here but it's ok because the actual hash is unused by blockifier
            } else {
                None
            }
        } else {
            None
        };

        // Perform validations
        let exec_context = ExecutionContext::new_in_block(Arc::clone(&self.backend), &pending_block_info)?;
        let mut validator = exec_context.tx_validator();
        let _ = validator.perform_validations(clone_account_tx(&tx), deploy_account_tx_hash.is_some());

        if !is_only_query(&tx) {
            // Finally, add it to the nonce chain for the account nonce
            let force = false;
            self.inner
                .write()
                .expect("Poisoned lock")
                .insert_tx(MempoolTransaction { tx, arrived_at, converted_class }, force)?
        }

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
    fn accept_invoke_tx(&self, tx: BroadcastedInvokeTransaction) -> Result<InvokeTransactionResult, Error> {
        let (tx, classes) = broadcasted_to_blockifier(
            BroadcastedTransaction::Invoke(tx),
            self.chain_id(),
            self.backend.chain_config().latest_protocol_version,
        )?;

        let res = InvokeTransactionResult { transaction_hash: transaction_hash(&tx) };
        self.accept_tx(tx, classes)?;
        Ok(res)
    }

    fn accept_declare_tx(&self, tx: BroadcastedDeclareTransaction) -> Result<DeclareTransactionResult, Error> {
        let (tx, classes) = broadcasted_to_blockifier(
            BroadcastedTransaction::Declare(tx),
            self.chain_id(),
            self.backend.chain_config().latest_protocol_version,
        )?;

        let res = DeclareTransactionResult {
            transaction_hash: transaction_hash(&tx),
            class_hash: declare_class_hash(&tx).expect("Created transaction should be declare"),
        };
        self.accept_tx(tx, classes)?;
        Ok(res)
    }

    fn accept_deploy_account_tx(
        &self,
        tx: BroadcastedDeployAccountTransaction,
    ) -> Result<DeployAccountTransactionResult, Error> {
        let (tx, classes) = broadcasted_to_blockifier(
            BroadcastedTransaction::DeployAccount(tx),
            self.chain_id(),
            self.backend.chain_config().latest_protocol_version,
        )?;

        let res = DeployAccountTransactionResult {
            transaction_hash: transaction_hash(&tx),
            contract_address: deployed_contract_address(&tx).expect("Created transaction should be deploy account"),
        };
        self.accept_tx(tx, classes)?;
        Ok(res)
    }

    /// Warning: A lock is held while a user-supplied function (extend) is run - Callers should be careful
    fn take_txs_chunk<I: Extend<MempoolTransaction> + 'static>(&self, dest: &mut I, n: usize) {
        let mut inner = self.inner.write().expect("Poisoned lock");
        inner.pop_next_chunk(dest, n)
    }

    fn take_tx(&self) -> Option<MempoolTransaction> {
        let mut inner = self.inner.write().expect("Poisoned lock");
        inner.pop_next()
    }

    /// Warning: A lock is taken while a user-supplied function (iterator stuff) is run - Callers should be careful
    fn re_add_txs<I: IntoIterator<Item = MempoolTransaction> + 'static>(&self, txs: I) {
        let mut inner = self.inner.write().expect("Poisoned lock");
        inner.re_add_txs(txs)
    }

    fn chain_id(&self) -> Felt {
        Felt::from_bytes_be_slice(format!("{}", self.backend.chain_config().chain_id).as_bytes())
    }
}

pub(crate) fn is_only_query(tx: &AccountTransaction) -> bool {
    match tx {
        AccountTransaction::Declare(tx) => tx.only_query(),
        AccountTransaction::DeployAccount(tx) => tx.only_query,
        AccountTransaction::Invoke(tx) => tx.only_query,
    }
}

pub(crate) fn contract_addr(tx: &AccountTransaction) -> ContractAddress {
    match tx {
        AccountTransaction::Declare(tx) => tx.tx.sender_address(),
        AccountTransaction::DeployAccount(tx) => tx.contract_address,
        AccountTransaction::Invoke(tx) => tx.tx.sender_address(),
    }
}

pub(crate) fn nonce(tx: &AccountTransaction) -> Nonce {
    match tx {
        AccountTransaction::Declare(tx) => tx.tx.nonce(),
        AccountTransaction::DeployAccount(tx) => tx.tx.nonce(),
        AccountTransaction::Invoke(tx) => tx.tx.nonce(),
    }
}

pub(crate) fn tx_hash(tx: &AccountTransaction) -> TransactionHash {
    match tx {
        AccountTransaction::Declare(tx) => tx.tx_hash,
        AccountTransaction::DeployAccount(tx) => tx.tx_hash,
        AccountTransaction::Invoke(tx) => tx.tx_hash,
    }
}

// AccountTransaction does not implement Clone :(
pub(crate) fn clone_account_tx(tx: &AccountTransaction) -> AccountTransaction {
    match tx {
        // Declare has a private field :(
        AccountTransaction::Declare(tx) => AccountTransaction::Declare(match tx.only_query() {
            // These should never fail
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
    }
}
