use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::Starknet;
use dc_exec::{block_context, execute_transactions, execution_result_to_fee_estimate, execution_result_to_tx_trace};
use dp_transactions::broadcasted_to_blockifier;
use starknet_core::types::{BlockId, BroadcastedTransaction, SimulatedTransaction, SimulationFlag};

use super::trace_transaction::FALLBACK_TO_SEQUENCER_WHEN_VERSION_BELOW;

pub async fn simulate_transactions(
    starknet: &Starknet,
    block_id: BlockId,
    transactions: Vec<BroadcastedTransaction>,
    simulation_flags: Vec<SimulationFlag>,
) -> StarknetRpcResult<Vec<SimulatedTransaction>> {
    let block_info = starknet.get_block_info(block_id)?;

    if block_info.header().protocol_version < FALLBACK_TO_SEQUENCER_WHEN_VERSION_BELOW {
        return Err(StarknetRpcApiError::UnsupportedTxnVersion);
    }
    let chain_id = starknet.chain_id();
    let block_context = block_context(block_info.header(), &chain_id);

    let charge_fee = !simulation_flags.contains(&SimulationFlag::SkipFeeCharge);
    let validate = !simulation_flags.contains(&SimulationFlag::SkipValidate);

    let user_transactions = transactions
        .into_iter()
        .map(|tx| broadcasted_to_blockifier(tx, chain_id))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| StarknetRpcApiError::ErrUnexpectedError {
            data: format!("Failed to convert broadcasted transaction to blockifier: {e}"),
        })?;

    let execution_resuls = execute_transactions(
        starknet.clone_backend(),
        vec![],
        user_transactions.into_iter(),
        &block_context,
        charge_fee,
        validate,
    )?;

    let simulated_transactions = execution_resuls
        .iter()
        .map(|result| {
            Ok(SimulatedTransaction {
                transaction_trace: execution_result_to_tx_trace(result).map_err(|e| {
                    StarknetRpcApiError::ErrUnexpectedError {
                        data: format!("Converting execution infos to tx trace: {e}"),
                    }
                })?,
                fee_estimation: execution_result_to_fee_estimate(result, &block_context),
            })
        })
        .collect::<Result<Vec<_>, StarknetRpcApiError>>()?;

    Ok(simulated_transactions)
}
