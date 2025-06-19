use super::trace_transaction::EXECUTION_UNSUPPORTED_BELOW_VERSION;
use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::utils::ResultExt;
use crate::Starknet;
use blockifier::transaction::account_transaction::ExecutionFlags;
use blockifier::transaction::transaction_execution::Transaction as BTransaction;
use mc_exec::{execution_result_to_tx_trace, ExecutionContext};
use mp_block::BlockId;
use mp_rpc::{BroadcastedTxn, SimulateTransactionsResult, SimulationFlag};
use mp_transactions::{BroadcastedTransactionExt, ToBlockifierError};
use starknet_api::executable_transaction::AccountTransaction as ApiAccountTransaction;
use starknet_api::transaction::Transaction as ApiTransaction;
use std::sync::Arc;

pub async fn simulate_transactions(
    starknet: &Starknet,
    block_id: BlockId,
    transactions: Vec<BroadcastedTxn>,
    simulation_flags: Vec<SimulationFlag>,
) -> StarknetRpcResult<Vec<SimulateTransactionsResult>> {
    let block_info = starknet.get_block_info(&block_id)?;
    let starknet_version = *block_info.protocol_version();

    if starknet_version < EXECUTION_UNSUPPORTED_BELOW_VERSION {
        return Err(StarknetRpcApiError::unsupported_txn_version());
    }
    let exec_context = ExecutionContext::new_at_block_end(Arc::clone(&starknet.backend), &block_info)?;

    let charge_fee = !simulation_flags.contains(&SimulationFlag::SkipFeeCharge);
    let validate = !simulation_flags.contains(&SimulationFlag::SkipValidate);

    let user_transactions = transactions
        .into_iter()
        .map(|tx| {
            let only_query = tx.is_query();
            let (api_tx, _) = tx.into_starknet_api(starknet.chain_id(), starknet_version)?;
            let tx_hash = api_tx.tx_hash();
            let class_info = match &api_tx {
                ApiAccountTransaction::Declare(declare_tx) => Some(declare_tx.class_info.to_owned()),
                _ => None,
            };
            let deployed_contract_address = match &api_tx {
                ApiAccountTransaction::DeployAccount(deploy_account_tx) => Some(deploy_account_tx.contract_address()),
                _ => None,
            };
            let tx = match api_tx {
                ApiAccountTransaction::Declare(declare_tx) => ApiTransaction::Declare(declare_tx.tx),
                ApiAccountTransaction::DeployAccount(deploy_account_tx) => {
                    ApiTransaction::DeployAccount(deploy_account_tx.tx)
                }
                ApiAccountTransaction::Invoke(invoke_tx) => ApiTransaction::Invoke(invoke_tx.tx),
            };
            let execution_flags = ExecutionFlags { only_query, charge_fee, validate, strict_nonce_check: true };
            Ok(BTransaction::from_api(
                tx,
                tx_hash,
                class_info,
                /* paid_fee_on_l1 */ None,
                deployed_contract_address,
                execution_flags,
            )?)
        })
        .collect::<Result<Vec<_>, ToBlockifierError>>()
        .or_internal_server_error("Failed to convert broadcasted transaction to blockifier")?;

    let execution_resuls = exec_context.re_execute_transactions([], user_transactions)?;

    let simulated_transactions = execution_resuls
        .iter()
        .map(|result| {
            Ok(SimulateTransactionsResult {
                transaction_trace: execution_result_to_tx_trace(result)
                    .or_internal_server_error("Converting execution infos to tx trace")?,
                fee_estimation: exec_context.execution_result_to_fee_estimate(result),
            })
        })
        .collect::<Result<Vec<_>, StarknetRpcApiError>>()?;

    Ok(simulated_transactions)
}
