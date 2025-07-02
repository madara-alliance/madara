use blockifier::transaction::account_transaction::ExecutionFlags;
use blockifier::transaction::objects::TransactionExecutionResult;
use blockifier::transaction::transaction_execution::Transaction as BTransaction;
use starknet_api::executable_transaction::AccountTransaction as ApiAccountTransaction;
use starknet_api::transaction::Transaction as ApiTransaction;

pub fn tx_api_to_blockifier(
    tx: ApiAccountTransaction,
    execution_flags: ExecutionFlags,
) -> TransactionExecutionResult<BTransaction> {
    let tx_hash = tx.tx_hash();

    let class_info = match &tx {
        ApiAccountTransaction::Declare(declare_tx) => Some(declare_tx.class_info.to_owned()),
        _ => None,
    };

    let deployed_contract_address = match &tx {
        ApiAccountTransaction::DeployAccount(deploy_account_tx) => Some(deploy_account_tx.contract_address()),
        _ => None,
    };

    let tx = match tx {
        ApiAccountTransaction::Declare(declare_tx) => ApiTransaction::Declare(declare_tx.tx),
        ApiAccountTransaction::DeployAccount(deploy_account_tx) => ApiTransaction::DeployAccount(deploy_account_tx.tx),
        ApiAccountTransaction::Invoke(invoke_tx) => ApiTransaction::Invoke(invoke_tx.tx),
    };

    BTransaction::from_api(
        tx,
        tx_hash,
        class_info,
        /* paid_fee_on_l1 */ None,
        deployed_contract_address,
        execution_flags,
    )
}
