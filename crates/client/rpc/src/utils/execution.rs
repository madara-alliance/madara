use std::sync::Arc;

use blockifier::context::{BlockContext, FeeTokenAddresses, TransactionContext};
use blockifier::execution::entry_point::{CallEntryPoint, CallType, EntryPointExecutionContext};
use blockifier::execution::errors::EntryPointExecutionError;
use blockifier::fee::gas_usage::estimate_minimal_gas_vector;
use blockifier::state::cached_state::{CachedState, GlobalContractCache};
use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::errors::TransactionExecutionError;
use blockifier::transaction::objects::{
    DeprecatedTransactionInfo, GasVector, HasRelatedFeeType, TransactionExecutionInfo, TransactionInfo,
};
use blockifier::transaction::transaction_execution::Transaction;
use blockifier::transaction::transactions::{ExecutableTransaction, L1HandlerTransaction};
use blockifier::versioned_constants::VersionedConstants;
use mc_db::storage_handler;
use mp_felt::Felt252Wrapper;
use mp_genesis_config::{ETH_TOKEN_ADDR, STRK_TOKEN_ADDR};
use mp_simulations::{SimulationFlagForEstimateFee, SimulationFlags};
use sp_blockchain::HeaderBackend;
use sp_runtime::traits::Block as BlockT;
use starknet_api::core::{ContractAddress, EntryPointSelector};
use starknet_api::deprecated_contract_class::EntryPointType;
use starknet_api::hash::StarkHash;
use starknet_api::transaction::Calldata;
use starknet_core::types::{FeeEstimate, PriceUnit};
use starknet_ff::FieldElement;

use super::blockifier_state_adapter::BlockifierStateAdapter;
use crate::errors::StarknetRpcApiError;
use crate::get_block_by_block_hash;

pub fn block_context<B, C>(
    client: &C,
    substrate_block_hash: <B as BlockT>::Hash,
) -> Result<BlockContext, StarknetRpcApiError>
where
    B: BlockT,
    C: HeaderBackend<B>,
{
    let block = get_block_by_block_hash(client, substrate_block_hash).map_err(|e| {
        log::error!("Failed to retrieve block by block hash: {e}");
        StarknetRpcApiError::BlockNotFound
    })?;
    let block_header = block.header();

    // safe unwrap because address is always valid and static
    let fee_token_address = FeeTokenAddresses {
        strk_fee_token_address: StarkHash::new_unchecked(STRK_TOKEN_ADDR.0.to_bytes_be()).try_into().unwrap(),
        eth_fee_token_address: StarkHash::new_unchecked(ETH_TOKEN_ADDR.0.to_bytes_be()).try_into().unwrap(),
    };
    let chain_id = starknet_api::core::ChainId("SN_MAIN".to_string());

    Ok(block_header.into_block_context(fee_token_address, chain_id))
}

pub fn re_execute_transactions(
    transactions_before: Vec<Transaction>,
    transactions_to_trace: Vec<Transaction>,
    block_context: &BlockContext,
) -> Result<Vec<TransactionExecutionInfo>, TransactionExecutionError> {
    let charge_fee = block_context.block_info().gas_prices.eth_l1_gas_price.get() != 1;
    let mut cached_state = init_cached_state(block_context);

    transactions_before
        .into_iter()
        .map(|tx| tx.execute(&mut cached_state, block_context, charge_fee, true))
        .collect::<Result<Vec<_>, _>>()?;

    let transactions_exec_infos = transactions_to_trace
        .into_iter()
        .map(|tx| tx.execute(&mut cached_state, block_context, charge_fee, true))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(transactions_exec_infos)
}

pub fn simulate_transactions(
    transactions: Vec<AccountTransaction>,
    simulation_flags: &SimulationFlags,
    block_context: &BlockContext,
) -> Result<Vec<TransactionExecutionInfo>, TransactionExecutionError> {
    let mut cached_state = init_cached_state(block_context);

    let tx_execution_results = transactions
        .into_iter()
        .map(|tx| tx.execute(&mut cached_state, block_context, simulation_flags.charge_fee, simulation_flags.validate))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(tx_execution_results)
}

/// Call a smart contract function.
pub fn call_contract(
    address: ContractAddress,
    function_selector: EntryPointSelector,
    calldata: Calldata,
    block_context: &BlockContext,
) -> Result<Vec<Felt252Wrapper>, ()> {
    // Get class hash
    let class_hash = storage_handler::contract_data().get_class_hash(&address).map_err(|_| ())?;

    let entrypoint = CallEntryPoint {
        class_hash,
        code_address: None,
        entry_point_type: EntryPointType::External,
        entry_point_selector: function_selector,
        calldata,
        storage_address: address,
        caller_address: ContractAddress::default(),
        call_type: CallType::Call,
        initial_gas: VersionedConstants::latest_constants().tx_initial_gas(),
    };

    let mut resources = cairo_vm::vm::runners::cairo_runner::ExecutionResources::default();
    let mut entry_point_execution_context = EntryPointExecutionContext::new_invoke(
        Arc::new(TransactionContext {
            block_context: block_context.clone(),
            tx_info: TransactionInfo::Deprecated(DeprecatedTransactionInfo::default()),
        }),
        false,
    )
    .map_err(|_| ())?;

    match entrypoint.execute(
        &mut BlockifierStateAdapter::new(block_context.block_info().block_number.0),
        &mut resources,
        &mut entry_point_execution_context,
    ) {
        Ok(v) => {
            log::debug!("Successfully called a smart contract function: {:?}", v);
            let result = v.execution.retdata.0.iter().map(|x| (*x).into()).collect();
            Ok(result)
        }
        Err(e) => {
            log::error!("failed to call smart contract {:?}", e);
            Err(())
        }
    }
}

pub fn estimate_fee(
    transactions: Vec<AccountTransaction>,
    simulation_flags: &[SimulationFlagForEstimateFee],
    block_context: &BlockContext,
) -> Result<Vec<FeeEstimate>, TransactionExecutionError> {
    let transactions_len = transactions.len();

    let mut fees = Vec::with_capacity(transactions_len);

    // TODO: the vector of flags should be for each transaction
    for tx in transactions {
        for flag in simulation_flags.iter() {
            let execution_info = execute_fee_transaction(tx.clone(), flag.clone(), block_context)?;
            fees.push(execution_info);
        }
    }

    Ok(fees)
}

pub fn estimate_message_fee(
    message: L1HandlerTransaction,
    block_context: &BlockContext,
) -> Result<FeeEstimate, TransactionExecutionError> {
    let mut cached_state = init_cached_state(block_context);

    let tx_execution_infos = message.clone().execute(&mut cached_state, block_context, true, true)?;

    // TODO: implement this
    // if !tx_execution_infos.is_reverted() {}

    let unit = match message.fee_type() {
        blockifier::transaction::objects::FeeType::Strk => PriceUnit::Fri,
        blockifier::transaction::objects::FeeType::Eth => PriceUnit::Wei,
    };

    let fee = FeeEstimate {
        gas_consumed: Felt252Wrapper::from(*tx_execution_infos.actual_resources.0.get("l1_gas_usage").unwrap()).into(),
        gas_price: FieldElement::ZERO,
        data_gas_consumed: tx_execution_infos.da_gas.l1_data_gas.into(),
        data_gas_price: FieldElement::ZERO,
        overall_fee: tx_execution_infos.actual_fee.0.into(),
        unit,
    };
    Ok(fee)
}

fn execute_fee_transaction(
    transaction: AccountTransaction,
    simulation_flags: SimulationFlagForEstimateFee,
    block_context: &BlockContext,
) -> Result<FeeEstimate, TransactionExecutionError> {
    let mut cached_state = init_cached_state(block_context);

    let fee_type = transaction.fee_type();

    let gas_price = block_context.block_info().gas_prices.get_gas_price_by_fee_type(&fee_type).get();
    let data_gas_price = block_context.block_info().gas_prices.get_data_gas_price_by_fee_type(&fee_type).get();
    let unit = match fee_type {
        blockifier::transaction::objects::FeeType::Strk => PriceUnit::Fri,
        blockifier::transaction::objects::FeeType::Eth => PriceUnit::Wei,
    };

    let minimal_l1_gas_amount_vector = estimate_minimal_gas_vector(block_context, &transaction)
        .map_err(TransactionExecutionError::TransactionPreValidationError);

    let tx_info: Result<
        blockifier::transaction::objects::TransactionExecutionInfo,
        blockifier::transaction::errors::TransactionExecutionError,
    > = transaction.execute(&mut cached_state, block_context, false, simulation_flags.skip_validate).and_then(
        |mut tx_info| {
            if tx_info.actual_fee.0 == 0 {
                tx_info.actual_fee =
                    blockifier::fee::fee_utils::calculate_tx_fee(&tx_info.actual_resources, block_context, &fee_type)?;
            }

            Ok(tx_info)
        },
    );

    match tx_info {
        Ok(tx_info) => {
            if let Some(_revert_error) = tx_info.revert_error {
                return Err(TransactionExecutionError::ExecutionError {
                    error: EntryPointExecutionError::InternalError("Transaction reverted".to_string()),
                    storage_address: ContractAddress::default(),
                    selector: EntryPointSelector::default(),
                });
            }

            let fee_estimate = from_tx_info_and_gas_price(
                &tx_info,
                gas_price,
                data_gas_price,
                unit,
                minimal_l1_gas_amount_vector.expect("Minimal gas vector error"),
            );
            Ok(fee_estimate)
        }
        Err(_error) => Err(TransactionExecutionError::ExecutionError {
            error: EntryPointExecutionError::InternalError("Execution error".to_string()),
            storage_address: ContractAddress::default(),
            selector: EntryPointSelector::default(),
        }),
    }
}

pub fn from_tx_info_and_gas_price(
    tx_info: &TransactionExecutionInfo,
    gas_price: u128,
    data_gas_price: u128,
    unit: PriceUnit,
    minimal_l1_gas_amount_vector: GasVector,
) -> FeeEstimate {
    let data_gas_consumed = tx_info.da_gas.l1_data_gas;
    let data_gas_fee = data_gas_consumed.saturating_mul(data_gas_price);
    let gas_consumed = tx_info.actual_fee.0.saturating_sub(data_gas_fee) / gas_price.max(1);

    let minimal_gas_consumed = minimal_l1_gas_amount_vector.l1_gas;
    let minimal_data_gas_consumed = minimal_l1_gas_amount_vector.l1_data_gas;

    let gas_consumed = gas_consumed.max(minimal_gas_consumed);
    let data_gas_consumed = data_gas_consumed.max(minimal_data_gas_consumed);
    let overall_fee =
        gas_consumed.saturating_mul(gas_price).saturating_add(data_gas_consumed.saturating_mul(data_gas_price));

    FeeEstimate {
        gas_consumed: gas_consumed.into(),
        gas_price: gas_price.into(),
        data_gas_consumed: data_gas_consumed.into(),
        data_gas_price: data_gas_price.into(),
        overall_fee: overall_fee.into(),
        unit,
    }
}

fn init_cached_state(block_context: &BlockContext) -> CachedState<BlockifierStateAdapter> {
    let block_number = block_context.block_info().block_number.0;
    CachedState::new(BlockifierStateAdapter::new(block_number), GlobalContractCache::new(10))
}
