use super::AddTransactionProvider;
use crate::{errors::StarknetRpcApiError, utils::display_internal_server_error};
use jsonrpsee::core::{async_trait, RpcResult};
use mc_mempool::Mempool;
use mc_mempool::MempoolProvider;
use mp_chain_config::StarknetVersion;
use mp_class::ConvertedClass;
use mp_transactions::broadcasted_to_blockifier;
use starknet_core::types::{
    BroadcastedDeclareTransaction, BroadcastedDeployAccountTransaction, BroadcastedInvokeTransaction,
    DeclareTransactionResult, DeployAccountTransactionResult, InvokeTransactionResult,
};
use std::sync::Arc;

/// This [`AddTransactionProvider`] adds the received transactions to a mempool.
pub struct MempoolAddTxProvider {
    mempool: Arc<Mempool>,
}

impl MempoolAddTxProvider {
    pub fn new(mempool: Arc<Mempool>) -> Self {
        Self { mempool }
    }
}

fn make_err(err: mc_mempool::Error) -> StarknetRpcApiError {
    if err.is_internal() {
        display_internal_server_error(format!("{err:#}"));
        StarknetRpcApiError::InternalServerError
    } else {
        StarknetRpcApiError::ValidationFailure { error: format!("{err:#}") }
    }
}

#[async_trait]
impl AddTransactionProvider for MempoolAddTxProvider {
    async fn add_declare_transaction(
        &self,
        declare_transaction: BroadcastedDeclareTransaction,
    ) -> RpcResult<DeclareTransactionResult> {
        Ok(self.mempool.accept_declare_tx(declare_transaction).map_err(make_err)?)
    }
    async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTransaction,
    ) -> RpcResult<DeployAccountTransactionResult> {
        Ok(self.mempool.accept_deploy_account_tx(deploy_account_transaction).map_err(make_err)?)
    }
    async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTransaction,
    ) -> RpcResult<InvokeTransactionResult> {
        Ok(self.mempool.accept_invoke_tx(invoke_transaction).map_err(make_err)?)
    }
}

fn transaction_hash(tx: &Transaction) -> Felt {
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

async fn add_tx_to_mempool(
    mempool: &Arc<Mempool>,
    tx: Transaction,
    converted_class: Option<ConvertedClass>,
) -> RpcResult<()> {
    let Transaction::AccountTransaction(tx) = tx else {
        bail_internal_server_error!("Created transaction should be an account transaction")
    };

    mempool
        .accept_account_tx(tx, converted_class)
        .map_err(|err| StarknetRpcApiError::TxnExecutionError { tx_index: 0, error: format!("{err:#}") })?;
    Ok(())
}

async fn add_declare_transaction(
    mempool: &Arc<Mempool>,
    declare_transaction: BroadcastedDeclareTransaction,
) -> RpcResult<DeclareTransactionResult> {
    let (tx, classes) = broadcasted_to_blockifier(
        BroadcastedTransaction::Declare(declare_transaction),
        mempool.chain_id(),
        Default::default(),
    )
    .map_err(|err| StarknetRpcApiError::TxnExecutionError { tx_index: 0, error: format!("{err:#}") })?;

    let res = DeclareTransactionResult {
        transaction_hash: transaction_hash(&tx),
        class_hash: declare_class_hash(&tx).expect("Created transaction should be declare"),
    };
    add_tx_to_mempool(mempool, tx, classes).await?;
    Ok(res)
}
async fn add_deploy_account_transaction(
    mempool: &Arc<Mempool>,
    deploy_account_transaction: BroadcastedDeployAccountTransaction,
) -> RpcResult<DeployAccountTransactionResult> {
    let (tx, classes) = broadcasted_to_blockifier(
        BroadcastedTransaction::DeployAccount(deploy_account_transaction),
        mempool.chain_id(),
        StarknetVersion::LATEST,
    )
    .map_err(|err| StarknetRpcApiError::TxnExecutionError { tx_index: 0, error: format!("{err:#}") })?;

    let res = DeployAccountTransactionResult {
        transaction_hash: transaction_hash(&tx),
        contract_address: deployed_contract_address(&tx).expect("Created transaction should be deploy account"),
    };
    add_tx_to_mempool(mempool, tx, classes).await?;
    Ok(res)
}
async fn add_invoke_transaction(
    mempool: &Arc<Mempool>,
    invoke_transaction: BroadcastedInvokeTransaction,
) -> RpcResult<InvokeTransactionResult> {
    let (tx, classes) = broadcasted_to_blockifier(
        BroadcastedTransaction::Invoke(invoke_transaction),
        mempool.chain_id(),
        StarknetVersion::LATEST,
    )
    .map_err(|err| StarknetRpcApiError::TxnExecutionError { tx_index: 0, error: format!("{err:#}") })?;

    let res = InvokeTransactionResult { transaction_hash: transaction_hash(&tx) };
    add_tx_to_mempool(mempool, tx, classes).await?;
    Ok(res)
}
