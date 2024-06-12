use std::sync::Arc;

use anyhow::Result;
use blockifier::context::{BlockContext, FeeTokenAddresses, TransactionContext};
use blockifier::execution::entry_point::{CallEntryPoint, CallType, EntryPointExecutionContext};
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
use dc_db::storage_handler::StorageView;
use dp_block::DeoxysBlockInfo;
use dp_felt::Felt252Wrapper;
use dp_simulations::SimulationFlags;
use jsonrpsee::core::RpcResult;
use starknet_api::core::{ContractAddress, EntryPointSelector};
use starknet_api::deprecated_contract_class::EntryPointType;
use starknet_api::hash::StarkHash;
use starknet_api::transaction::Calldata;
use starknet_core::types::{FeeEstimate, PriceUnit};
use starknet_ff::FieldElement;

use super::blockifier_state_adapter::BlockifierStateAdapter;
use crate::errors::StarknetRpcApiError;
use crate::utils::ResultExt;
use crate::Starknet;

// 0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7
pub const ETH_TOKEN_ADDR: FieldElement =
    FieldElement::from_mont([4380532846569209554, 17839402928228694863, 17240401758547432026, 418961398025637529]);

// 0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d
pub const STRK_TOKEN_ADDR: FieldElement =
    FieldElement::from_mont([16432072983745651214, 1325769094487018516, 5134018303144032807, 468300854463065062]);

pub fn block_context(_client: &Starknet, block_info: &DeoxysBlockInfo) -> Result<BlockContext, StarknetRpcApiError> {
    let block_header = block_info.header();

    // safe unwrap because address is always valid and static
    let fee_token_address = FeeTokenAddresses {
        strk_fee_token_address: StarkHash::new_unchecked(STRK_TOKEN_ADDR.to_bytes_be()).try_into().unwrap(),
        eth_fee_token_address: StarkHash::new_unchecked(ETH_TOKEN_ADDR.to_bytes_be()).try_into().unwrap(),
    };
    let chain_id = starknet_api::core::ChainId("SN_MAIN".to_string());

    Ok(block_header.into_block_context(fee_token_address, chain_id))
}

pub fn re_execute_transactions(
    starknet: &Starknet,
    transactions_before: Vec<Transaction>,
    transactions_to_trace: Vec<Transaction>,
    block_context: &BlockContext,
) -> Result<Vec<TransactionExecutionInfo>, TransactionExecutionError> {
    let charge_fee = block_context.block_info().gas_prices.eth_l1_gas_price.get() != 1;
    let mut cached_state = init_cached_state(starknet, block_context);

    for tx in transactions_before {
        tx.execute(&mut cached_state, block_context, charge_fee, true)?;
    }

    let transactions_exec_infos = transactions_to_trace
        .into_iter()
        .map(|tx| tx.execute(&mut cached_state, block_context, charge_fee, true))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(transactions_exec_infos)
}

pub fn simulate_transactions(
    starknet: &Starknet,
    transactions: Vec<AccountTransaction>,
    simulation_flags: &SimulationFlags,
    block_context: &BlockContext,
) -> Result<Vec<TransactionExecutionInfo>, TransactionExecutionError> {
    let mut cached_state = init_cached_state(starknet, block_context);

    let tx_execution_results = transactions
        .into_iter()
        .map(|tx| tx.execute(&mut cached_state, block_context, simulation_flags.charge_fee, simulation_flags.validate))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(tx_execution_results)
}

/// Call a smart contract function.
pub fn call_contract(
    starknet: &Starknet,
    address: ContractAddress,
    function_selector: EntryPointSelector,
    calldata: Calldata,
    block_context: &BlockContext,
) -> RpcResult<Vec<Felt252Wrapper>> {
    // Get class hash
    let class_hash = starknet
        .backend
        .contract_class_hash()
        .get(&address)
        .or_internal_server_error("Error getting contract class hash")?;

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
    .map_err(|err| {
        log::error!("Transaction execution error: {err}");
        StarknetRpcApiError::TxnExecutionError
    })?;

    let res = entrypoint
        .execute(
            &mut BlockifierStateAdapter::new(Arc::clone(&starknet.backend), block_context.block_info().block_number.0),
            &mut resources,
            &mut entry_point_execution_context,
        )
        .map_err(|err| {
            log::error!("Entry point execution error: {err}");
            StarknetRpcApiError::TxnExecutionError
        })?;

    log::debug!("Successfully called a smart contract function: {:?}", res);
    let result = res.execution.retdata.0.iter().map(|x| (*x).into()).collect();
    Ok(result)
}

pub fn estimate_fee(
    starknet: &Starknet,
    transactions: Vec<AccountTransaction>,
    validate: bool,
    block_context: &BlockContext,
) -> Result<Vec<FeeEstimate>, TransactionExecutionError> {
    let fees = transactions
        .into_iter()
        .map(|tx| execute_fee_transaction(starknet, tx.clone(), validate, block_context))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(fees)
}

pub fn estimate_message_fee(
    starknet: &Starknet,
    message: L1HandlerTransaction,
    block_context: &BlockContext,
) -> Result<FeeEstimate, TransactionExecutionError> {
    let mut cached_state = init_cached_state(starknet, block_context);

    let tx_execution_infos = message.clone().execute(&mut cached_state, block_context, true, true)?;

    // TODO: implement this
    // if !tx_execution_infos.is_reverted() {}

    let unit = match message.fee_type() {
        blockifier::transaction::objects::FeeType::Strk => PriceUnit::Fri,
        blockifier::transaction::objects::FeeType::Eth => PriceUnit::Wei,
    };

    // TODO: implement this
    // if !tx_execution_infos.is_reverted() {}

    let fee = FeeEstimate {
        gas_consumed: FieldElement::from(
            tx_execution_infos.actual_resources.0.get("l1_gas_usage").cloned().unwrap_or_default(),
        ),
        gas_price: FieldElement::ZERO,
        data_gas_consumed: tx_execution_infos.da_gas.l1_data_gas.into(),
        data_gas_price: FieldElement::ZERO,
        overall_fee: tx_execution_infos.actual_fee.0.into(),
        unit,
    };
    Ok(fee)
}

fn execute_fee_transaction(
    starknet: &Starknet,
    transaction: AccountTransaction,
    validate: bool,
    block_context: &BlockContext,
) -> Result<FeeEstimate, TransactionExecutionError> {
    let mut cached_state = init_cached_state(starknet, block_context);

    let fee_type = transaction.fee_type();

    let gas_price = block_context.block_info().gas_prices.get_gas_price_by_fee_type(&fee_type).get();
    let data_gas_price = block_context.block_info().gas_prices.get_data_gas_price_by_fee_type(&fee_type).get();
    let unit = match fee_type {
        blockifier::transaction::objects::FeeType::Strk => PriceUnit::Fri,
        blockifier::transaction::objects::FeeType::Eth => PriceUnit::Wei,
    };

    let minimal_l1_gas_amount_vector = estimate_minimal_gas_vector(block_context, &transaction)
        .map_err(TransactionExecutionError::TransactionPreValidationError)?;

    let tx_info: Result<
        blockifier::transaction::objects::TransactionExecutionInfo,
        blockifier::transaction::errors::TransactionExecutionError,
    > = transaction.execute(&mut cached_state, block_context, false, validate).and_then(|mut tx_info| {
        if tx_info.actual_fee.0 == 0 {
            tx_info.actual_fee =
                blockifier::fee::fee_utils::calculate_tx_fee(&tx_info.actual_resources, block_context, &fee_type)?;
        }

        Ok(tx_info)
    });

    match tx_info {
        Ok(tx_info) => {
            let fee_estimate =
                from_tx_info_and_gas_price(&tx_info, gas_price, data_gas_price, unit, minimal_l1_gas_amount_vector);
            Ok(fee_estimate)
        }
        Err(error) => Err(error),
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

fn init_cached_state(starknet: &Starknet, block_context: &BlockContext) -> CachedState<BlockifierStateAdapter> {
    let block_number = block_context.block_info().block_number.0;
    CachedState::new(
        BlockifierStateAdapter::new(Arc::clone(&starknet.backend), block_number - 1), // TODO(panic): check if this -1 operation can overflow!!
        GlobalContractCache::new(16),
    )
}
