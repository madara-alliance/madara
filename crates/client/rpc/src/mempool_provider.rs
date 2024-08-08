use std::sync::Arc;

use super::providers::AddTransactionProvider;
use crate::{bail_internal_server_error, errors::StarknetRpcApiError};
use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::transaction_execution::Transaction;
use dc_mempool::Mempool;
use dp_class::ConvertedClass;
use dp_transactions::broadcasted_to_blockifier;
use jsonrpsee::core::{async_trait, RpcResult};
use starknet_core::types::{
    BroadcastedDeclareTransaction, BroadcastedDeployAccountTransaction, BroadcastedInvokeTransaction,
    BroadcastedTransaction, DeclareTransactionResult, DeployAccountTransactionResult, Felt, InvokeTransactionResult,
};

/// This [`AddTransactionProvider`] adds the received transactions to a mempool.
pub struct MempoolProvider {
    mempool: Arc<Mempool>,
}

impl MempoolProvider {
    pub fn new(mempool: Arc<Mempool>) -> Self {
        Self { mempool }
    }
}

#[async_trait]
impl AddTransactionProvider for MempoolProvider {
    async fn add_declare_transaction(
        &self,
        declare_transaction: BroadcastedDeclareTransaction,
    ) -> RpcResult<DeclareTransactionResult> {
        Ok(add_declare_transaction(&self.mempool, declare_transaction).await?)
    }
    async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTransaction,
    ) -> RpcResult<DeployAccountTransactionResult> {
        Ok(add_deploy_account_transaction(&self.mempool, deploy_account_transaction).await?)
    }
    async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTransaction,
    ) -> RpcResult<InvokeTransactionResult> {
        Ok(add_invoke_transaction(&self.mempool, invoke_transaction).await?)
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
        .await
        .map_err(|err| StarknetRpcApiError::TxnExecutionError { tx_index: 0, error: format!("{err:#}") })?;
    Ok(())
}

async fn add_declare_transaction(
    mempool: &Arc<Mempool>,
    declare_transaction: BroadcastedDeclareTransaction,
) -> RpcResult<DeclareTransactionResult> {
    let (tx, classes) =
        broadcasted_to_blockifier(BroadcastedTransaction::Declare(declare_transaction), mempool.chain_id(), None)
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
        None,
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
    let (tx, classes) =
        broadcasted_to_blockifier(BroadcastedTransaction::Invoke(invoke_transaction), mempool.chain_id(), None)
            .map_err(|err| StarknetRpcApiError::TxnExecutionError { tx_index: 0, error: format!("{err:#}") })?;

    let res = InvokeTransactionResult { transaction_hash: transaction_hash(&tx) };
    add_tx_to_mempool(mempool, tx, classes).await?;
    Ok(res)
}
