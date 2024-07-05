use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::utils::ResultExt;
use crate::Starknet;
use dc_exec::{execution_result_to_tx_trace, ExecutionContext};
use dp_transactions::broadcasted_to_blockifier;
use starknet_core::types::{BlockId, BroadcastedTransaction, SimulatedTransaction, SimulationFlag};

use super::trace_transaction::FALLBACK_TO_SEQUENCER_WHEN_VERSION_BELOW;

pub async fn simulate_transactions(
    starknet: &Starknet,
    block_id: BlockId,
    transactions: Vec<BroadcastedTransaction>,
    simulation_flags: Vec<SimulationFlag>,
) -> StarknetRpcResult<Vec<SimulatedTransaction>> {
    let block_info = starknet.get_block_info(&block_id)?;

    if block_info.protocol_version() < &FALLBACK_TO_SEQUENCER_WHEN_VERSION_BELOW {
        return Err(StarknetRpcApiError::UnsupportedTxnVersion);
    }
    let exec_context = ExecutionContext::new(&starknet.backend, &block_info)?;

    let charge_fee = !simulation_flags.contains(&SimulationFlag::SkipFeeCharge);
    let validate = !simulation_flags.contains(&SimulationFlag::SkipValidate);

    let user_transactions = transactions
        .into_iter()
        .map(|tx| broadcasted_to_blockifier(tx, starknet.chain_id()))
        .collect::<Result<Vec<_>, _>>()
        .or_internal_server_error("Failed to convert broadcasted transaction to blockifier")?;

    let execution_resuls = exec_context.execute_transactions([], user_transactions, charge_fee, validate)?;

    let simulated_transactions = execution_resuls
        .iter()
        .map(|result| {
            Ok(SimulatedTransaction {
                transaction_trace: execution_result_to_tx_trace(result)
                    .or_internal_server_error("Converting execution infos to tx trace")?,
                fee_estimation: exec_context.execution_result_to_fee_estimate(result),
            })
        })
        .collect::<Result<Vec<_>, StarknetRpcApiError>>()?;

    Ok(simulated_transactions)
}
