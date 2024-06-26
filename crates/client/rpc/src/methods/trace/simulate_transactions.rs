use blockifier::transaction::objects::{FeeType, HasRelatedFeeType};
use dp_simulations::SimulationFlags;
use dp_transactions::broadcasted_to_blockifier;
use jsonrpsee::core::RpcResult;
use starknet_core::types::{
    BlockId, BroadcastedTransaction, FeeEstimate, PriceUnit, SimulatedTransaction, SimulationFlag,
};

use super::lib::ConvertCallInfoToExecuteInvocationError;
use super::utils::tx_execution_infos_to_tx_trace;
use crate::errors::StarknetRpcApiError;
use crate::utils::execution::{block_context, ExecutionResult};
use crate::utils::ResultExt;
use crate::{utils, Starknet};

pub async fn simulate_transactions(
    starknet: &Starknet,
    block_id: BlockId,
    transactions: Vec<BroadcastedTransaction>,
    simulation_flags: Vec<SimulationFlag>,
) -> RpcResult<Vec<SimulatedTransaction>> {
    let block_info = starknet.get_block_info(block_id)?;
    let block_context = block_context(starknet, &block_info)?;
    let chain_id = starknet.chain_id()?;

    let simulation_flags = SimulationFlags {
        validate: !simulation_flags.contains(&SimulationFlag::SkipValidate),
        charge_fee: !simulation_flags.contains(&SimulationFlag::SkipFeeCharge),
    };

    let user_transactions = transactions
        .into_iter()
        .map(|tx| broadcasted_to_blockifier(tx, chain_id))
        .collect::<Result<Vec<_>, _>>()
        .or_internal_server_error("Failed to convert BroadcastedTransaction to UserTransaction")?;

    let fee_types = user_transactions.iter().map(|tx| tx.fee_type()).collect::<Vec<_>>();

    let execution_result =
        utils::execution::simulate_transactions(starknet, user_transactions, &simulation_flags, &block_context)
            .map_err(|_| StarknetRpcApiError::ContractError)?;

    let simulated_transactions = tx_execution_infos_to_simulated_transactions(&execution_result, fee_types)
        .map_err(StarknetRpcApiError::from)?;

    Ok(simulated_transactions)
}

fn tx_execution_infos_to_simulated_transactions(
    transactions_executions_results: &[ExecutionResult],
    fee_types: Vec<FeeType>,
) -> Result<Vec<SimulatedTransaction>, ConvertCallInfoToExecuteInvocationError> {
    let mut results = vec![];

    for (tx_result, fee_type) in transactions_executions_results.iter().zip(fee_types.into_iter()) {
        let gas =
            tx_result.execution_info.execute_call_info.as_ref().map(|x| x.execution.gas_consumed).unwrap_or_default();
        let fee = tx_result.execution_info.actual_fee.0;
        let price = if gas > 0 { fee / gas as u128 } else { 0 };

        let gas_consumed = gas.into();
        let gas_price = price.into();
        let overall_fee = fee.into();

        let unit = match fee_type {
            FeeType::Eth => PriceUnit::Wei,
            FeeType::Strk => PriceUnit::Fri,
        };

        let data_gas_consumed = tx_result.execution_info.da_gas.l1_data_gas.into();
        let data_gas_price = tx_result.execution_info.da_gas.l1_gas.into();

        let transaction_trace = tx_execution_infos_to_tx_trace(tx_result)?;

        results.push(SimulatedTransaction {
            transaction_trace,
            fee_estimation: FeeEstimate {
                gas_consumed,
                data_gas_consumed,
                data_gas_price,
                gas_price,
                overall_fee,
                unit,
            },
        });
    }
    Ok(results)
}
