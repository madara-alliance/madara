use blockifier::transaction::objects::{FeeType, HasRelatedFeeType, TransactionExecutionInfo};
use jsonrpsee::core::RpcResult;
use mp_hashers::HasherT;
use mp_simulations::SimulationFlags;
use mp_transactions::from_broadcasted_transactions::ToAccountTransaction;
use mp_transactions::TxType;
use mp_types::block::DBlockT;
use pallet_starknet_runtime_api::StarknetRuntimeApi;
use sc_client_api::{Backend, BlockBackend, StorageProvider};
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_core::types::{
    BlockId, BroadcastedTransaction, FeeEstimate, PriceUnit, SimulatedTransaction, SimulationFlag,
};

use super::lib::ConvertCallInfoToExecuteInvocationError;
use super::utils::{block_number_by_id, tx_execution_infos_to_tx_trace};
use crate::errors::StarknetRpcApiError;
use crate::utils::execution::block_context;
use crate::{utils, Starknet};

pub async fn simulate_transactions<BE, C, H>(
    starknet: &Starknet<BE, C, H>,
    block_id: BlockId,
    transactions: Vec<BroadcastedTransaction>,
    simulation_flags: Vec<SimulationFlag>,
) -> RpcResult<Vec<SimulatedTransaction>>
where
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT>,
    H: HasherT + Send + Sync + 'static,
{
    let substrate_block_hash = starknet.substrate_block_hash_from_starknet_block(block_id)?;

    let block_context = block_context(starknet.client.as_ref(), substrate_block_hash)?;
    let block_number = block_number_by_id(block_id)?;

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
        itertools::process_results(tx_type_and_tx_iterator, |iter| iter.unzip::<_, _, Vec<_>, Vec<_>>()).map_err(
            |e| {
                log::error!("Failed to convert BroadcastedTransaction to UserTransaction: {e}");
                StarknetRpcApiError::InternalServerError
            },
        )?;

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
