use super::trace_transaction::EXECUTION_UNSUPPORTED_BELOW_VERSION;
use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::utils::ResultExt;
use crate::Starknet;
use mc_exec::{execution_result_to_tx_trace, ExecutionContext};
use mp_block::BlockId;
use mp_transactions::BroadcastedTransactionExt;
use starknet_types_core::felt::Felt;
use starknet_types_rpc::{BroadcastedTxn, SimulateTransactionsResult, SimulationFlag};
use std::sync::Arc;

pub async fn simulate_transactions(
    starknet: &Starknet,
    block_id: BlockId,
    transactions: Vec<BroadcastedTxn<Felt>>,
    simulation_flags: Vec<SimulationFlag>,
) -> StarknetRpcResult<Vec<SimulateTransactionsResult<Felt>>> {
    let block_info = starknet.get_block_info(&block_id)?;
    let starknet_version = *block_info.protocol_version();

    if starknet_version < EXECUTION_UNSUPPORTED_BELOW_VERSION {
        return Err(StarknetRpcApiError::UnsupportedTxnVersion);
    }
    let exec_context = ExecutionContext::new_at_block_end(Arc::clone(&starknet.backend), &block_info)?;

    let charge_fee = !simulation_flags.contains(&SimulationFlag::SkipFeeCharge);
    let validate = !simulation_flags.contains(&SimulationFlag::SkipValidate);

    let user_transactions = transactions
        .into_iter()
        .map(|tx| tx.into_blockifier(starknet.chain_id(), starknet_version, validate, charge_fee).map(|(tx, _)| tx))
        .collect::<Result<Vec<_>, _>>()
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
