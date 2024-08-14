use blockifier::blockifier::stateful_validator::StatefulValidatorError;
use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::transactions::DeclareTransaction;
use blockifier::transaction::transactions::DeployAccountTransaction;
use blockifier::transaction::transactions::InvokeTransaction;
use dc_db::db_block_id::DbBlockId;
use dc_db::DeoxysBackend;
use dc_db::DeoxysStorageError;
use dc_exec::ExecutionContext;
use dp_block::BlockId;
use dp_block::BlockTag;
use dp_block::DeoxysPendingBlockInfo;
use dp_class::ConvertedClass;
use header::make_pending_header;
use inner::MempoolInner;
pub use inner::{ArrivedAtTimestamp, MempoolTransaction};
pub use l1::L1DataProvider;
use starknet_api::core::{ContractAddress, Nonce};
use starknet_api::transaction::TransactionHash;
use starknet_types_core::felt::Felt;
use std::sync::Arc;
use std::sync::RwLock;

pub mod block_production;
mod close_block;
pub mod header;
mod inner;
mod l1;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Storage error: {0:#}")]
    StorageError(#[from] DeoxysStorageError),
    #[error("No genesis block in storage")]
    NoGenesis,
    #[error("Validation error: {0:#}")]
    Validation(#[from] StatefulValidatorError),
    #[error(transparent)]
    InnerMempool(#[from] inner::TxInsersionError),
    #[error(transparent)]
    Exec(#[from] dc_exec::Error),
}

pub struct Mempool {
    backend: Arc<DeoxysBackend>,
    l1_data_provider: Arc<dyn L1DataProvider>,
    inner: RwLock<MempoolInner>,
}

impl Mempool {
    pub fn new(backend: Arc<DeoxysBackend>, l1_data_provider: Arc<dyn L1DataProvider>) -> Self {
        Mempool { backend, l1_data_provider, inner: Default::default() }
    }

    pub fn accept_account_tx(
        &self,
        tx: AccountTransaction,
        converted_class: Option<ConvertedClass>,
    ) -> Result<(), Error> {
        // The timestamp *does not* take the transaction validation time into account.
        let arrived_at = ArrivedAtTimestamp::now();

        // Get pending block.
        let pending_block_info = if let Some(block) = self.backend.get_block_info(&DbBlockId::Pending)? {
            block
        } else {
            // No current pending block, we'll make an unsaved empty one for the sake of validating this tx.
            let parent_block_hash =
                self.backend.get_block_hash(&BlockId::Tag(BlockTag::Latest))?.ok_or(Error::NoGenesis)?;
            DeoxysPendingBlockInfo::new(
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
        let exec_context = ExecutionContext::new(Arc::clone(&self.backend), &pending_block_info)?;
        let mut validator = exec_context.tx_validator();
        validator.perform_validations(clone_account_tx(&tx), deploy_account_tx_hash)?;

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

    pub fn take_txs_chunk(&self, dest: &mut Vec<MempoolTransaction>, n: usize) {
        let mut inner = self.inner.write().expect("Poisoned lock");
        inner.pop_next_chunk(dest, n)
    }

    pub fn take_tx(&self) -> Option<MempoolTransaction> {
        let mut inner = self.inner.write().expect("Poisoned lock");
        inner.pop_next()
    }

    pub fn re_add_txs(&self, txs: Vec<MempoolTransaction>) {
        let mut inner = self.inner.write().expect("Poisoned lock");
        inner.re_add_txs(txs)
    }

    pub fn chain_id(&self) -> Felt {
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
            true => DeclareTransaction::new_for_query(tx.tx.clone(), tx.tx_hash, tx.class_info.clone()).unwrap(),
            false => DeclareTransaction::new(tx.tx.clone(), tx.tx_hash, tx.class_info.clone()).unwrap(),
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
