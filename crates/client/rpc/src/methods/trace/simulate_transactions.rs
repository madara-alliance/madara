use blockifier::transaction::objects::{FeeType, HasRelatedFeeType, TransactionExecutionInfo};
use jsonrpsee::core::RpcResult;
use mp_simulations::SimulationFlags;
use mp_transactions::from_broadcasted_transactions::ToAccountTransaction;
use mp_transactions::TxType;
use starknet_core::types::{
    BlockId, BroadcastedTransaction, FeeEstimate, PriceUnit, SimulatedTransaction, SimulationFlag,
};

use super::lib::ConvertCallInfoToExecuteInvocationError;
use super::utils::tx_execution_infos_to_tx_trace;
use crate::errors::StarknetRpcApiError;
use crate::utils::execution::block_context;
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
    let block_number = block_info.block_n();

    let simulation_flags = SimulationFlags {
        validate: !simulation_flags.contains(&SimulationFlag::SkipValidate),
        charge_fee: !simulation_flags.contains(&SimulationFlag::SkipFeeCharge),
    };

    let tx_type_and_tx_iterator = transactions.into_iter().map(|tx| match tx {
        BroadcastedTransaction::Invoke(_) => tx.to_account_transaction().map(|tx| (TxType::Invoke, tx)),
        BroadcastedTransaction::Declare(_) => tx.to_account_transaction().map(|tx| (TxType::Declare, tx)),
        BroadcastedTransaction::DeployAccount(_) => tx.to_account_transaction().map(|tx| (TxType::DeployAccount, tx)),
    });
    let (tx_types, user_transactions) =
        itertools::process_results(tx_type_and_tx_iterator, |iter| iter.unzip::<_, _, Vec<_>, Vec<_>>())
            .or_internal_server_error("Failed to convert BroadcastedTransaction to UserTransaction")?;

    let fee_types = user_transactions.iter().map(|tx| tx.fee_type()).collect::<Vec<_>>();

    let res = utils::execution::simulate_transactions(user_transactions, &simulation_flags, &block_context)
        .map_err(|_| StarknetRpcApiError::ContractError)?;

    let simulated_transactions = tx_execution_infos_to_simulated_transactions(tx_types, res, block_number, fee_types)
        .map_err(StarknetRpcApiError::from)?;

    Ok(simulated_transactions)
}

fn tx_execution_infos_to_simulated_transactions(
    tx_types: Vec<TxType>,
    transaction_execution_results: Vec<TransactionExecutionInfo>,
    block_number: u64,
    fee_types: Vec<FeeType>,
) -> Result<Vec<SimulatedTransaction>, ConvertCallInfoToExecuteInvocationError> {
    let mut results = vec![];

    for ((tx_type, res), fee_type) in
        tx_types.into_iter().zip(transaction_execution_results.into_iter()).zip(fee_types.into_iter())
    {
        let transaction_trace = tx_execution_infos_to_tx_trace(tx_type, &res, block_number)?;
        let gas = res.execute_call_info.as_ref().map(|x| x.execution.gas_consumed).unwrap_or_default();
        let fee = res.actual_fee.0;
        let price = if gas > 0 { fee / gas as u128 } else { 0 };

        let gas_consumed = gas.into();
        let gas_price = price.into();
        let overall_fee = fee.into();

        let unit = match fee_type {
            FeeType::Eth => PriceUnit::Wei,
            FeeType::Strk => PriceUnit::Fri,
        };

        let data_gas_consumed = res.da_gas.l1_data_gas.into();
        let data_gas_price = res.da_gas.l1_gas.into();

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
