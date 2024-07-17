use std::borrow::Cow;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::SystemTime;

use blockifier::blockifier::stateful_validator::StatefulValidatorError;
use blockifier::transaction::account_transaction::AccountTransaction;
use dc_db::db_block_id::DbBlockId;
use dc_db::storage_handler::DeoxysStorageError;
use dc_db::DeoxysBackend;
use dc_exec::ExecutionContext;
use dp_block::header::PendingHeader;
use dp_block::{
    BlockId, BlockTag, DeoxysBlockInner, DeoxysMaybePendingBlock, DeoxysMaybePendingBlockInfo, DeoxysPendingBlockInfo,
};
use inner::MempoolInner;
use starknet_api::core::{ContractAddress, Nonce};

mod inner;
mod l1;

pub use l1::L1DataProvider;
use starknet_api::transaction::TransactionHash;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Storage error: {0:#}")]
    StorageError(#[from] DeoxysStorageError),
    #[error("No genesis block in storage")]
    NoGenesis,
    #[error("Internal error: {0}")]
    Internal(Cow<'static, str>),
    #[error("Validation error: {0:#}")]
    Validation(#[from] StatefulValidatorError),
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

    /// This function creates the pending block if it is not found.
    fn pending_block(&self) -> Result<DeoxysMaybePendingBlock, Error> {
        match self.backend.get_block(&DbBlockId::Pending)? {
            Some(block) => Ok(block),
            None => {
                // No pending block: we create one :)

                let block_info =
                    self.backend.get_block_info(&BlockId::Tag(BlockTag::Latest))?.ok_or(Error::NoGenesis)?;
                let block_info = block_info.as_nonpending().ok_or(Error::Internal("Latest block is pending".into()))?;

                Ok(DeoxysMaybePendingBlock {
                    info: DeoxysMaybePendingBlockInfo::Pending(DeoxysPendingBlockInfo {
                        header: PendingHeader {
                            parent_block_hash: block_info.block_hash,
                            sequencer_address: **self.backend.chain_config().sequencer_address,
                            block_timestamp: SystemTime::now()
                                .duration_since(SystemTime::UNIX_EPOCH)
                                .expect("Current time is before the unix timestamp")
                                .as_secs(),
                            protocol_version: self.backend.chain_config().latest_protocol_version,
                            l1_gas_price: self.l1_data_provider.get_gas_prices(),
                            l1_da_mode: self.l1_data_provider.get_da_mode(),
                        },
                        tx_hashes: vec![],
                    }),
                    inner: DeoxysBlockInner { transactions: vec![], receipts: vec![] },
                })
            }
        }
    }

    pub fn accept_account_tx(&self, tx: AccountTransaction) -> Result<(), Error> {
        // Get pending block
        let pending_block_info = self.pending_block()?;

        // If the contract has been deployed for the same block is is invoked, we need to skip validations.
        // NB: the lock is NOT taken the entire time the tx is being validated. As such, the deploy tx
        //  may appear during that time - but it is not a problem.
        let deploy_account_tx_hash = if let AccountTransaction::Invoke(tx) = tx {
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
        let exec_context = ExecutionContext::new(&self.backend, &pending_block_info.info)?;
        let validator = exec_context.tx_validator();
        validator.perform_validations(tx, deploy_account_tx_hash)?;

        if !is_only_query(&tx) {
            // Finally, add it to the nonce chain for the account nonce
            self.inner.write().insert_account_tx()
        }

        Ok(())
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

}
