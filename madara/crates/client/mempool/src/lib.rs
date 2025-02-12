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
use mp_rpc::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn,
    BroadcastedTxn, ClassAndTxnHash, ContractAndTxnHash,
};
use mp_transactions::BroadcastedDeclareTransactionV0;
use mp_transactions::BroadcastedTransactionExt;
use mp_transactions::L1HandlerTransaction;
use mp_transactions::L1HandlerTransactionResult;
use mp_transactions::ToBlockifierError;
use starknet_api::core::{ContractAddress, Nonce};
use starknet_api::transaction::TransactionHash;
use starknet_api::StarknetApiError;
use starknet_types_core::felt::Felt;
use std::collections::{BTreeMap, VecDeque};
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

#[cfg(any(test, feature = "testing"))]
#[allow(unused)]
pub(crate) trait CheckInvariants {
    /// Validates the invariants of this struct.
    ///
    /// This method should cause a panic if these invariants are not met.
    fn check_invariants(&self);
}

#[cfg_attr(test, mockall::automock)]
pub trait MempoolProvider: Send + Sync {
    fn tx_accept_invoke(&self, tx: BroadcastedInvokeTxn) -> Result<AddInvokeTransactionResult, MempoolError>;
    fn tx_accept_declare_v0(&self, tx: BroadcastedDeclareTransactionV0) -> Result<ClassAndTxnHash, MempoolError>;
    fn tx_accept_declare(&self, tx: BroadcastedDeclareTxn) -> Result<ClassAndTxnHash, MempoolError>;
    fn tx_accept_deploy_account(&self, tx: BroadcastedDeployAccountTxn) -> Result<ContractAndTxnHash, MempoolError>;
    fn tx_accept_l1_handler(
        &self,
        tx: L1HandlerTransaction,
        paid_fees_on_l1: u128,
    ) -> Result<L1HandlerTransactionResult, MempoolError>;
    fn txs_take_chunk(&self, dest: &mut VecDeque<MempoolTransaction>, n: usize);
    fn tx_take(&mut self) -> Option<MempoolTransaction>;
    fn tx_mark_included(&self, contract_address: &Felt);
    fn txs_re_add(
        &self,
        txs: VecDeque<MempoolTransaction>,
        consumed_txs: Vec<MempoolTransaction>,
    ) -> Result<(), MempoolError>;
    fn chain_id(&self) -> Felt;
}

pub struct Mempool {
    backend: Arc<MadaraBackend>,
    l1_data_provider: Arc<dyn L1DataProvider>,
    inner: RwLock<MempoolInner>,
    metrics: MempoolMetrics,
    nonce_cache: RwLock<BTreeMap<Felt, Nonce>>,
}

impl Mempool {
    pub fn new(backend: Arc<MadaraBackend>, l1_data_provider: Arc<dyn L1DataProvider>, limits: MempoolLimits) -> Self {
        Mempool {
            backend,
            l1_data_provider,
            inner: RwLock::new(MempoolInner::new(limits)),
            metrics: MempoolMetrics::register(),
            nonce_cache: RwLock::new(BTreeMap::new()),
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
            // No current pending block, we'll make an unsaved empty one for
            // the sake of validating this tx.
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

            // TODO: should we update this to store only if the mempool accepts
            // this transaction?
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

    /// Determines the status of a transaction based on the address of the
    /// contract sending it and its nonce.
    ///
    /// Several mechanisms are used to keep the latest nonce for a contract in
    /// sync with the state of the node, even if the db is not being updated
    /// fast enough. These include keeping a mapping of the latest nonce for
    /// contracts to be included in the upcoming block as well as checking the
    /// state of the mempool to see if previous transactions are marked as
    /// ready.
    fn retrieve_nonce_info(&self, sender_address: Felt, nonce: Felt) -> Result<NonceInfo, MempoolError> {
        let nonce = Nonce(nonce);
        let nonce_next = nonce.try_increment()?;

        let nonce_prev_check = || {
            // We don't need an underflow check here as nonces are incremental
            // and non negative, so there is no nonce s.t nonce != nonce_target,
            // nonce < nonce_target & nonce = 0
            let nonce_prev = Nonce(nonce.0 - Felt::ONE);
            let nonce_prev_ready = self.inner.read().expect("Poisoned lock").nonce_is_ready(sender_address, nonce_prev);

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
        let nonce_cached = self.nonce_cache.read().expect("Poisoned lock").get(&sender_address).cloned();

        if let Some(nonce_cached) = nonce_cached {
            match nonce.cmp(&nonce_cached) {
                std::cmp::Ordering::Less => Err(MempoolError::StorageError(MadaraStorageError::InvalidNonce)),
                std::cmp::Ordering::Equal => Ok(NonceInfo::ready(nonce, nonce_next)),
                std::cmp::Ordering::Greater => nonce_prev_check(),
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
                std::cmp::Ordering::Less => Err(MempoolError::StorageError(MadaraStorageError::InvalidNonce)),
                std::cmp::Ordering::Equal => Ok(NonceInfo::ready(nonce, nonce_next)),
                std::cmp::Ordering::Greater => nonce_prev_check(),
            }
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
    fn tx_accept_invoke(&self, tx: BroadcastedInvokeTxn) -> Result<AddInvokeTransactionResult, MempoolError> {
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
    fn tx_accept_declare_v0(&self, tx: BroadcastedDeclareTransactionV0) -> Result<ClassAndTxnHash, MempoolError> {
        let (btx, class) = tx.into_blockifier(self.chain_id(), self.backend.chain_config().latest_protocol_version)?;

        let res = ClassAndTxnHash {
            transaction_hash: transaction_hash(&btx),
            class_hash: declare_class_hash(&btx).expect("Created transaction should be declare"),
        };

        self.accept_tx(btx, class, ArrivedAtTimestamp::now(), NonceInfo::default())?;
        Ok(res)
    }

    #[tracing::instrument(skip(self), fields(module = "Mempool"))]
    fn tx_accept_l1_handler(
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
    fn tx_accept_declare(&self, tx: BroadcastedDeclareTxn) -> Result<ClassAndTxnHash, MempoolError> {
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
    fn tx_accept_deploy_account(&self, tx: BroadcastedDeployAccountTxn) -> Result<ContractAndTxnHash, MempoolError> {
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
    fn txs_take_chunk(&self, dest: &mut VecDeque<MempoolTransaction>, n: usize) {
        let mut inner = self.inner.write().expect("Poisoned lock");
        let mut nonce_cache = self.nonce_cache.write().expect("Poisoned lock");

        let from = dest.len();
        inner.pop_next_chunk(dest, n);

        for mempool_tx in dest.iter().skip(from) {
            let contract_address = mempool_tx.contract_address().to_felt();
            let nonce_next = mempool_tx.nonce_next;
            nonce_cache.insert(contract_address, nonce_next);
        }
    }

    #[tracing::instrument(skip(self), fields(module = "Mempool"))]
    fn tx_take(&mut self) -> Option<MempoolTransaction> {
        if let Some(mempool_tx) = self.inner.write().expect("Poisoned lock").pop_next() {
            let contract_address = mempool_tx.contract_address().to_felt();
            let nonce_next = mempool_tx.nonce_next;
            self.nonce_cache.write().expect("Poisoned lock").insert(contract_address, nonce_next);

            Some(mempool_tx)
        } else {
            None
        }
    }

    #[tracing::instrument(skip(self, contract_address), fields(module = "Mempool"))]
    fn tx_mark_included(&self, contract_address: &Felt) {
        let removed = self.nonce_cache.write().expect("Poisoned lock").remove(contract_address);
        debug_assert!(removed.is_some());
    }

    /// Warning: A lock is taken while a user-supplied function (iterator stuff)
    /// is run - Callers should be careful This is called by the block
    /// production after a batch of transaction is executed. Mark the consumed
    /// txs as consumed, and re-add the transactions that are not consumed in
    /// the mempool.
    #[tracing::instrument(skip(self, txs, consumed_txs), fields(module = "Mempool"))]
    fn txs_re_add(
        &self,
        txs: VecDeque<MempoolTransaction>,
        consumed_txs: Vec<MempoolTransaction>,
    ) -> Result<(), MempoolError> {
        let mut inner = self.inner.write().expect("Poisoned lock");
        let mut nonce_cache = self.nonce_cache.write().expect("Poisoned lock");

        for tx in txs.iter() {
            // Nonce cache is invalidated upon re-insertion into the mempool
            // as it is currently possible for these transactions to be
            // removed if their age exceeds the limit. In the future, we
            // might want to update this if we make it so only pending
            // transactions can be removed this way.
            let contract_address = **tx.contract_address();
            let removed = nonce_cache.remove(&contract_address);
            debug_assert!(removed.is_some());
        }

        let hashes = consumed_txs.iter().map(|tx| tx.tx_hash()).collect::<Vec<_>>();
        inner.re_add_txs(txs, consumed_txs);
        drop(inner);

        for tx_hash in hashes {
            self.backend.remove_mempool_transaction(&tx_hash.to_felt())?;
        }

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

    use mc_db::mempool_db::NonceStatus;
    use mp_block::{MadaraBlockInfo, MadaraBlockInner, MadaraMaybePendingBlock, MadaraMaybePendingBlockInfo};
    use mp_state_update::{NonceUpdate, StateDiff};

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

    #[rstest::fixture]
    fn tx_deploy_v1_valid(
        #[default(Felt::ZERO)] contract_address: Felt,
    ) -> blockifier::transaction::transaction_execution::Transaction {
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
        )
    }

    #[rstest::fixture]
    fn tx_l1_handler_valid(
        #[default(Felt::ZERO)] contract_address: Felt,
    ) -> blockifier::transaction::transaction_execution::Transaction {
        blockifier::transaction::transaction_execution::Transaction::L1HandlerTransaction(
            blockifier::transaction::transactions::L1HandlerTransaction {
                tx: starknet_api::transaction::L1HandlerTransaction {
                    contract_address: ContractAddress::try_from(contract_address).unwrap(),
                    ..Default::default()
                },
                tx_hash: starknet_api::transaction::TransactionHash::default(),
                paid_fee_on_l1: starknet_api::transaction::Fee::default(),
            },
        )
    }

    #[rstest::rstest]
    #[timeout(Duration::from_millis(1_000))]
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

    /// This test checks if a ready transaction is indeed inserted into the
    /// ready queue.
    #[rstest::rstest]
    #[timeout(Duration::from_millis(1_000))]
    fn mempool_accept_tx_ready(
        backend: Arc<mc_db::MadaraBackend>,
        l1_data_provider: Arc<MockL1DataProvider>,
        tx_account_v0_valid: blockifier::transaction::transaction_execution::Transaction,
    ) {
        let sender_address = Felt::ZERO;
        let nonce = Nonce(Felt::ZERO);

        let mempool = Mempool::new(backend, l1_data_provider, MempoolLimits::for_testing());
        let arrived_at = ArrivedAtTimestamp::now();
        let result = mempool.accept_tx(tx_account_v0_valid, None, arrived_at, NonceInfo::default());
        assert_matches::assert_matches!(result, Ok(()));

        let inner = mempool.inner.read().expect("Poisoned lock");
        inner.check_invariants();

        assert!(inner.tx_intent_queue_ready.contains(&TransactionIntentReady {
            contract_address: Felt::ZERO,
            timestamp: arrived_at,
            nonce: Nonce(Felt::ZERO),
            nonce_next: Nonce(Felt::ONE),
            phantom: std::marker::PhantomData,
        }));

        assert!(inner.nonce_is_ready(sender_address, nonce));
    }

    #[rstest::rstest]
    #[timeout(Duration::from_millis(1_000))]
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

    /// This test makes sure that taking a transaction from the mempool works as
    /// intended.
    #[rstest::rstest]
    #[timeout(Duration::from_millis(1_000))]
    fn mempool_take_tx_pass(
        backend: Arc<mc_db::MadaraBackend>,
        l1_data_provider: Arc<MockL1DataProvider>,
        tx_account_v0_valid: blockifier::transaction::transaction_execution::Transaction,
    ) {
        let mut mempool = Mempool::new(backend, l1_data_provider, MempoolLimits::for_testing());
        let timestamp = ArrivedAtTimestamp::now();
        let result = mempool.accept_tx(tx_account_v0_valid, None, timestamp, NonceInfo::default());
        assert_matches::assert_matches!(result, Ok(()));

        let mempool_tx = mempool.tx_take().expect("Mempool should contain a transaction");
        assert_eq!(mempool_tx.arrived_at, timestamp);

        assert!(mempool.tx_take().is_none(), "It should not be possible to take a transaction from an empty mempool");

        mempool.inner.read().expect("Poisoned lock").check_invariants();
    }

    /// This test makes sure that all deploy account transactions inserted into
    /// [MempoolInner] are accounted for. Replacements are not taken into
    /// account.
    #[rstest::rstest]
    #[timeout(Duration::from_millis(1_000))]
    fn mempool_deploy_count(
        backend: Arc<mc_db::MadaraBackend>,
        l1_data_provider: Arc<MockL1DataProvider>,
        tx_deploy_v1_valid: blockifier::transaction::transaction_execution::Transaction,
    ) {
        let mempool = Mempool::new(backend, l1_data_provider, MempoolLimits::for_testing());

        let nonce_info = NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE));
        let mempool_tx = MempoolTransaction {
            tx: tx_deploy_v1_valid,
            arrived_at: ArrivedAtTimestamp::now(),
            converted_class: None,
            nonce: nonce_info.nonce,
            nonce_next: nonce_info.nonce_next,
        };
        let contract_address = mempool_tx.contract_address();

        let force = true;
        let update_limits = true;
        let result =
            mempool.inner.write().expect("Poisoned lock").insert_tx(mempool_tx, force, update_limits, nonce_info);
        assert_matches::assert_matches!(result, Ok(()));

        let inner = mempool.inner.read().expect("Poisoned lock");
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
    fn mempool_deploy_replace(
        backend: Arc<mc_db::MadaraBackend>,
        l1_data_provider: Arc<MockL1DataProvider>,
        tx_account_v0_valid: blockifier::transaction::transaction_execution::Transaction,
        tx_deploy_v1_valid: blockifier::transaction::transaction_execution::Transaction,
    ) {
        let mempool = Mempool::new(backend, l1_data_provider, MempoolLimits::for_testing());

        let nonce_info = NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE));
        let mempool_tx = MempoolTransaction {
            tx: tx_account_v0_valid,
            arrived_at: ArrivedAtTimestamp::now(),
            converted_class: None,
            nonce: nonce_info.nonce,
            nonce_next: nonce_info.nonce_next,
        };
        let contract_address = mempool_tx.contract_address();

        // We hack our way into the inner mempool to avoid having to generate a
        // valid deploy transaction since the outer mempool checks this
        let force = false;
        let update_limits = true;
        let result =
            mempool.inner.write().expect("Poisoned lock").insert_tx(mempool_tx, force, update_limits, nonce_info);
        assert_matches::assert_matches!(result, Ok(()));

        // We insert a first non-deploy tx. This should not update the count of
        // deploy transactions.
        let inner = mempool.inner.read().expect("Poisoned lock");
        assert!(!inner.deployed_contracts.contains(&contract_address));
        inner.check_invariants();
        drop(inner);

        let nonce_info = NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE));
        let mempool_tx = MempoolTransaction {
            tx: tx_deploy_v1_valid,
            arrived_at: ArrivedAtTimestamp::now(),
            converted_class: None,
            nonce: nonce_info.nonce,
            nonce_next: nonce_info.nonce_next,
        };
        let contract_address = mempool_tx.contract_address();

        // Now we replace the previous transaction with a deploy account tx
        let force = true;
        let update_limits = true;
        let result =
            mempool.inner.write().expect("Poisoned lock").insert_tx(mempool_tx, force, update_limits, nonce_info);
        assert_matches::assert_matches!(result, Ok(()));

        // This should have updated the count of deploy transactions.
        let inner = mempool.inner.read().expect("Poisoned lock");
        assert!(inner.deployed_contracts.contains(&contract_address));
        inner.check_invariants();
    }

    /// This test makes sure that replacing a deploy account transaction with a
    /// non-deploy account transaction reduces the deploy transaction count.
    ///
    /// > This bug was originally detected through proptesting
    #[rstest::rstest]
    #[timeout(Duration::from_millis(1_000))]
    fn mempool_deploy_replace2(
        backend: Arc<mc_db::MadaraBackend>,
        l1_data_provider: Arc<MockL1DataProvider>,
        tx_deploy_v1_valid: blockifier::transaction::transaction_execution::Transaction,
        tx_account_v0_valid: blockifier::transaction::transaction_execution::Transaction,
    ) {
        let mempool = Mempool::new(backend, l1_data_provider, MempoolLimits::for_testing());

        // First, we insert the deploy account transaction
        let nonce_info = NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE));
        let mempool_tx = MempoolTransaction {
            tx: tx_deploy_v1_valid,
            arrived_at: ArrivedAtTimestamp::now(),
            converted_class: None,
            nonce: nonce_info.nonce,
            nonce_next: nonce_info.nonce_next,
        };
        let contract_address = mempool_tx.contract_address();

        let force = true;
        let update_limits = true;
        let result =
            mempool.inner.write().expect("Poisoned lock").insert_tx(mempool_tx, force, update_limits, nonce_info);
        assert_matches::assert_matches!(result, Ok(()));

        let inner = mempool.inner.read().expect("Poisoned lock");
        assert!(inner.deployed_contracts.contains(&contract_address));
        inner.check_invariants();
        drop(inner);

        // Now we replace the previous transaction with a non-deploy account tx
        let nonce_info = NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE));
        let mempool_tx = MempoolTransaction {
            tx: tx_account_v0_valid,
            arrived_at: ArrivedAtTimestamp::now(),
            converted_class: None,
            nonce: nonce_info.nonce,
            nonce_next: nonce_info.nonce_next,
        };
        let contract_address = mempool_tx.contract_address();

        let force = true;
        let update_limits = true;
        let result =
            mempool.inner.write().expect("Poisoned lock").insert_tx(mempool_tx, force, update_limits, nonce_info);
        assert_matches::assert_matches!(result, Ok(()));

        // The deploy transaction count at that address should be 0
        let inner = mempool.inner.read().expect("Poisoned lock");
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
    fn mempool_deploy_remove_age_exceeded(
        backend: Arc<mc_db::MadaraBackend>,
        l1_data_provider: Arc<MockL1DataProvider>,
        tx_deploy_v1_valid: blockifier::transaction::transaction_execution::Transaction,
    ) {
        let mempool = Mempool::new(
            backend,
            l1_data_provider,
            MempoolLimits { max_age: Some(Duration::from_secs(3_600)), ..MempoolLimits::for_testing() },
        );

        // First, we insert the deploy account transaction
        let nonce_info = NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE));
        let mempool_tx = MempoolTransaction {
            tx: tx_deploy_v1_valid,
            arrived_at: ArrivedAtTimestamp::UNIX_EPOCH,
            converted_class: None,
            nonce: nonce_info.nonce,
            nonce_next: nonce_info.nonce_next,
        };
        let contract_address = mempool_tx.contract_address();

        let force = true;
        let update_limits = true;
        let result =
            mempool.inner.write().expect("Poisoned lock").insert_tx(mempool_tx, force, update_limits, nonce_info);
        assert_matches::assert_matches!(result, Ok(()));

        let inner = mempool.inner.read().expect("Poisoned lock");
        assert!(inner.deployed_contracts.contains(&contract_address));
        inner.check_invariants();
        drop(inner);

        // Next, we manually call `remove_age_exceeded_txs`
        mempool.inner.write().expect("Poisoned lock").remove_age_exceeded_txs();

        // This should have removed the deploy contract transaction and updated
        // the count at that address
        let inner = mempool.inner.read().expect("Poisoned lock");
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
        #[with(Felt::TWO)]
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
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending_by_nonce
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
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending_by_nonce
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
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready,
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending_by_nonce
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
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending_by_nonce
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
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending_by_nonce
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
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready,
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending_by_nonce
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
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready,
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending_by_nonce
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
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending_by_nonce
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
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending_by_nonce
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
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending_by_nonce
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
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending_by_nonce
        );

        // tx_new_3 should still be in the pending queue but tx_old_3 should not
        assert!(
            mempool
                .inner
                .read()
                .expect("Poisoned lock")
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
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready,
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending_by_nonce
        );
        assert!(
            !mempool
                .inner
                .read()
                .expect("Poisoned lock")
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
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready,
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending_by_nonce
        );

        // tx_old_4 should no longer be in the pending queue and since it was
        // the only transaction for that contract address, that pending queue
        // should have been emptied
        assert!(
            !mempool
                .inner
                .read()
                .expect("Poisoned lock")
                .tx_intent_queue_pending_by_nonce
                .contains_key(&**tx_old_4_mempool.contract_address()),
            "ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready,
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending_by_nonce
        );

        // Make sure we have not entered an invalid state.
        mempool.inner.read().expect("Poisoned lock").check_invariants();
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
    fn mempool_remove_aged_tx_pass_l1_handler(
        backend: Arc<mc_db::MadaraBackend>,
        l1_data_provider: Arc<MockL1DataProvider>,
        tx_l1_handler_valid: blockifier::transaction::transaction_execution::Transaction,
    ) {
        let mempool = Mempool::new(backend, l1_data_provider, MempoolLimits::for_testing());

        let nonce_info = NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE));
        let mempool_tx = MempoolTransaction {
            tx: tx_l1_handler_valid,
            arrived_at: ArrivedAtTimestamp::now(),
            converted_class: None,
            nonce: nonce_info.nonce,
            nonce_next: nonce_info.nonce_next,
        };

        let force = false;
        let update_limits = true;
        let result =
            mempool.inner.write().expect("Poisoned lock").insert_tx(mempool_tx, force, update_limits, nonce_info);
        assert_matches::assert_matches!(result, Ok(()));

        let inner = mempool.inner.read().expect("Poisoned lock");
        assert!(inner.nonce_is_ready(Felt::ZERO, Nonce(Felt::ZERO)));
        inner.check_invariants();
        drop(inner);

        // This should not loop!
        mempool.inner.write().expect("Poisoned lock").remove_age_exceeded_txs();
        let inner = mempool.inner.read().expect("Poisoned lock");
        assert!(inner.nonce_is_ready(Felt::ZERO, Nonce(Felt::ZERO)));
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
    #[rstest::rstest]
    #[timeout(Duration::from_millis(1_000))]
    fn mempool_readiness_check(
        backend: Arc<mc_db::MadaraBackend>,
        l1_data_provider: Arc<MockL1DataProvider>,
        #[from(tx_account_v0_valid)] tx_ready: blockifier::transaction::transaction_execution::Transaction,
        #[from(tx_account_v0_valid)] tx_pending: blockifier::transaction::transaction_execution::Transaction,
    ) {
        let mut mempool = Mempool::new(backend, l1_data_provider, MempoolLimits::for_testing());

        // Insert pending transaction

        let nonce_info = NonceInfo::pending(Nonce(Felt::ONE), Nonce(Felt::TWO));
        let timestamp_pending = ArrivedAtTimestamp::now();
        let result = mempool.accept_tx(tx_pending, None, timestamp_pending, nonce_info);
        assert_matches::assert_matches!(result, Ok(()));

        let inner = mempool.inner.read().expect("Poisoned lock");
        inner.check_invariants();
        inner
            .tx_intent_queue_pending_by_nonce
            .get(&Felt::ZERO)
            .expect("Mempool should have a pending queue for contract address ZERO")
            .get(&TransactionIntentPendingByNonce {
                contract_address: Felt::ZERO,
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

        let mempool_tx = mempool.tx_take().expect("Mempool contains a ready transaction!");

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
        assert!(inner.tx_intent_queue_pending_by_nonce.is_empty());

        drop(inner);

        // Take the next transaction. The pending transaction has been marked as
        // ready and should be returned here

        let mempool_tx = mempool.tx_take().expect("Mempool contains a ready transaction!");

        let inner = mempool.inner.read().expect("Poisoned lock");
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
    fn mempool_readiness_check_against_db(
        backend: Arc<mc_db::MadaraBackend>,
        l1_data_provider: Arc<MockL1DataProvider>,
        #[from(tx_account_v0_valid)] tx_1: blockifier::transaction::transaction_execution::Transaction,
        #[from(tx_account_v0_valid)] tx_2: blockifier::transaction::transaction_execution::Transaction,
    ) {
        let mempool = Mempool::new(backend, l1_data_provider, MempoolLimits::for_testing());

        // Insert the first transaction

        let nonce_info = mempool.retrieve_nonce_info(Felt::ZERO, Felt::ZERO).expect("Failed to retrieve nonce info");
        let timestamp_1 = ArrivedAtTimestamp::now();
        let result = mempool.accept_tx(tx_1, None, timestamp_1, nonce_info);
        assert_matches::assert_matches!(result, Ok(()));

        let inner = mempool.inner.read().expect("Poisoned lock");
        inner.check_invariants();
        let contains = inner.tx_intent_queue_ready.contains(&TransactionIntentReady {
            contract_address: Felt::ZERO,
            timestamp: timestamp_1,
            nonce: Nonce(Felt::ZERO),
            nonce_next: Nonce(Felt::ONE),
            phantom: Default::default(),
        });
        assert!(
            contains,
            "Mempool should contain transaction 1, ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready,
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending_by_nonce
        );

        assert_eq!(inner.tx_intent_queue_ready.len(), 1);

        drop(inner);

        // Insert the next transaction. It should be marked as ready even if it
        // does not have the correct nonce compared to the db (the test db
        // defaults to nonce 0 for all contracts) since the transaction before
        // it has already been marked as ready in the mempool.

        let nonce_info = mempool.retrieve_nonce_info(Felt::ZERO, Felt::ONE).expect("Failed to retrieve nonce info");
        let timestamp_2 = ArrivedAtTimestamp::now();
        let result = mempool.accept_tx(tx_2, None, timestamp_2, nonce_info);
        assert_matches::assert_matches!(result, Ok(()));

        let inner = mempool.inner.read().expect("Poisoned lock");
        inner.check_invariants();
        let contains = inner.tx_intent_queue_ready.contains(&TransactionIntentReady {
            contract_address: Felt::ZERO,
            timestamp: timestamp_2,
            nonce: Nonce(Felt::ONE),
            nonce_next: Nonce(Felt::TWO),
            phantom: Default::default(),
        });
        assert!(
            contains,
            "Mempool should contain transaction 2, ready transaction intents are: {:#?}\npending transaction intents are: {:#?}",
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready,
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending_by_nonce
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
    fn mempool_readiness_check_agains_nonce_cache(
        backend: Arc<mc_db::MadaraBackend>,
        l1_data_provider: Arc<MockL1DataProvider>,
    ) {
        let mempool = Mempool::new(backend, l1_data_provider, MempoolLimits::for_testing());

        let contract_address = Felt::ZERO;
        let nonce = Nonce(Felt::ONE);
        let nonce_next = nonce.try_increment().unwrap();
        mempool.nonce_cache.write().expect("Poisoned lock").insert(contract_address, nonce);

        // Despite the tx nonce being Felt::ONE, it should be marked as ready
        // since it is present in the nonce cache
        assert_eq!(mempool.retrieve_nonce_info(contract_address, *nonce).unwrap(), NonceInfo::ready(nonce, nonce_next));

        // We remove the nonce cache at that address
        mempool.tx_mark_included(&contract_address);

        // Now the nonce should be marked as pending
        assert_eq!(
            mempool.retrieve_nonce_info(contract_address, *nonce).unwrap(),
            NonceInfo::pending(nonce, nonce_next)
        );
    }

    /// This test makes sure that retrieve nonce_info has proper error handling
    /// and returns correct values based on the state of the db.
    #[rstest::rstest]
    #[timeout(Duration::from_millis(1_000))]
    fn mempool_retrieve_nonce_info(backend: Arc<mc_db::MadaraBackend>, l1_data_provider: Arc<MockL1DataProvider>) {
        let mempool = Mempool::new(Arc::clone(&backend), l1_data_provider, MempoolLimits::for_testing());

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
                None,
                None,
            )
            .expect("Failed to store block");

        // Transactions which meet the nonce in db should be marked as ready...

        assert_eq!(
            mempool.retrieve_nonce_info(Felt::ZERO, Felt::ZERO).unwrap(),
            NonceInfo { readiness: NonceStatus::Ready, nonce: Nonce(Felt::ZERO), nonce_next: Nonce(Felt::ONE) }
        );

        // ...otherwise they should be marked as pending

        assert_eq!(
            mempool.retrieve_nonce_info(Felt::ZERO, Felt::ONE).unwrap(),
            NonceInfo { readiness: NonceStatus::Pending, nonce: Nonce(Felt::ONE), nonce_next: Nonce(Felt::TWO) }
        );

        // Contract addresses which do not yet have a nonce store in db should
        // should not fail during nonce info retrieval

        assert_eq!(
            mempool.retrieve_nonce_info(Felt::ONE, Felt::ZERO).unwrap(),
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
                None,
                None,
            )
            .expect("Failed to store block");

        assert_matches::assert_matches!(
            mempool.retrieve_nonce_info(Felt::ZERO, Felt::ZERO),
            Err(MempoolError::StorageError(MadaraStorageError::InvalidNonce))
        );

        // We need to compute the next nonce inside retrieve nonce_info, so
        // passing Felt::MAX is not allowed.
        assert_matches::assert_matches!(
            mempool.retrieve_nonce_info(Felt::ZERO, Felt::MAX),
            Err(MempoolError::StarknetApi(StarknetApiError::OutOfRange { .. }))
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
    fn mempool_replace_pass(
        backend: Arc<mc_db::MadaraBackend>,
        l1_data_provider: Arc<MockL1DataProvider>,
        #[from(tx_account_v0_valid)] tx_1: blockifier::transaction::transaction_execution::Transaction,
        #[from(tx_account_v0_valid)] tx_2: blockifier::transaction::transaction_execution::Transaction,
        #[from(tx_account_v0_valid)]
        #[with(Felt::ONE)]
        tx_3: blockifier::transaction::transaction_execution::Transaction,
    ) {
        let mempool = Mempool::new(backend, l1_data_provider, MempoolLimits::for_testing());

        // Insert the first transaction

        let force = true;
        let update_tx_limits = true;

        let arrived_at = ArrivedAtTimestamp::now();
        let nonce_info = NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE));
        let tx_1_mempool = MempoolTransaction {
            tx: tx_1,
            arrived_at,
            converted_class: None,
            nonce: nonce_info.nonce,
            nonce_next: nonce_info.nonce_next,
        };
        let result = mempool.inner.write().expect("Poisoned lock").insert_tx(
            tx_1_mempool.clone(),
            force,
            update_tx_limits,
            nonce_info,
        );
        assert_matches::assert_matches!(result, Ok(()));

        let inner = mempool.inner.read().expect("Poisoned lock");
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
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready,
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending_by_nonce
        );

        assert_eq!(inner.tx_intent_queue_ready.len(), 1);

        inner.check_invariants();

        drop(inner);

        // Inserts the second transaction with the same nonce as the first once.
        // This should replace the first transaction.

        let force = true;
        let update_tx_limits = true;

        let arrived_at = ArrivedAtTimestamp::now();
        let nonce_info = NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE));
        let tx_2_mempool = MempoolTransaction {
            tx: tx_2,
            arrived_at,
            converted_class: None,
            nonce: nonce_info.nonce,
            nonce_next: nonce_info.nonce_next,
        };
        let result = mempool.inner.write().expect("Poisoned lock").insert_tx(
            tx_2_mempool.clone(),
            force,
            update_tx_limits,
            nonce_info,
        );
        assert_matches::assert_matches!(result, Ok(()));

        let inner = mempool.inner.read().expect("Poisoned lock");
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
            mempool.inner.read().expect("poisoned lock").tx_intent_queue_ready,
            mempool.inner.read().expect("poisoned lock").tx_intent_queue_pending_by_nonce
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
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready,
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending_by_nonce
        );

        assert_eq!(inner.tx_intent_queue_ready.len(), 1);

        inner.check_invariants();

        drop(inner);

        // Inserts the third transaction. This should not trigger a replacement
        // since it has a different contract address

        let force = true;
        let update_tx_limits = true;

        let arrived_at = ArrivedAtTimestamp::now();
        let nonce_info = NonceInfo::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE));
        let tx_3_mempool = MempoolTransaction {
            tx: tx_3,
            arrived_at,
            converted_class: None,
            nonce: nonce_info.nonce,
            nonce_next: nonce_info.nonce_next,
        };
        let result = mempool.inner.write().expect("Poisoned lock").insert_tx(
            tx_3_mempool.clone(),
            force,
            update_tx_limits,
            nonce_info.clone(),
        );
        assert_matches::assert_matches!(result, Ok(()));

        let inner = mempool.inner.read().expect("Poisoned lock");
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
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready,
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending_by_nonce
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
            mempool.inner.read().expect("poisoned lock").tx_intent_queue_ready,
            mempool.inner.read().expect("poisoned lock").tx_intent_queue_pending_by_nonce
        );

        assert_eq!(inner.tx_intent_queue_ready.len(), 2);

        inner.check_invariants();

        drop(inner);

        // Inserting a transaction again should fail if force if false. This
        // should not change the state of the mempool whatsoever!

        let force = false;
        let update_tx_limits = true;

        let result = mempool.inner.write().expect("Poisoned lock").insert_tx(
            tx_3_mempool.clone(),
            force,
            update_tx_limits,
            nonce_info,
        );
        assert_eq!(result, Err(TxInsertionError::DuplicateTxn));

        let inner = mempool.inner.read().expect("Poisoned lock");
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
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_ready,
            mempool.inner.read().expect("Poisoned lock").tx_intent_queue_pending_by_nonce
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
            mempool.inner.read().expect("poisoned lock").tx_intent_queue_ready,
            mempool.inner.read().expect("poisoned lock").tx_intent_queue_pending_by_nonce
        );

        assert_eq!(inner.tx_intent_queue_ready.len(), 2);

        inner.check_invariants();
    }
}
