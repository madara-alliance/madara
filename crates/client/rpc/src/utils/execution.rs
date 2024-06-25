use std::sync::Arc;

use anyhow::Result;
use blockifier::context::{BlockContext, FeeTokenAddresses, TransactionContext};
use blockifier::execution::entry_point::{CallEntryPoint, CallType, EntryPointExecutionContext};
use blockifier::fee::gas_usage::estimate_minimal_gas_vector;
use blockifier::state::cached_state::{CachedState, CommitmentStateDiff, GlobalContractCache};
use blockifier::state::state_api::State;
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
use dp_convert::ToFelt;
use dp_convert::ToStarkFelt;
use dp_simulations::SimulationFlags;
use dp_transactions::TxType;
use jsonrpsee::core::RpcResult;
use starknet_api::core::{ClassHash, ContractAddress, EntryPointSelector};
use starknet_api::deprecated_contract_class::EntryPointType;
use starknet_api::transaction::Calldata;
use starknet_api::transaction::TransactionHash;
use starknet_core::types::{FeeEstimate, PriceUnit};
use starknet_types_core::felt::Felt;

use super::blockifier_state_adapter::BlockifierStateAdapter;
use crate::errors::StarknetRpcApiError;
use crate::utils::ResultExt;
use crate::Starknet;

// 0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7
pub const ETH_TOKEN_ADDR: Felt =
    Felt::from_raw([418961398025637529, 17240401758547432026, 17839402928228694863, 4380532846569209554]);

// 0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d
pub const STRK_TOKEN_ADDR: Felt =
    Felt::from_raw([468300854463065062, 5134018303144032807, 1325769094487018516, 16432072983745651214]);

pub fn block_context(_client: &Starknet, block_info: &DeoxysBlockInfo) -> Result<BlockContext, StarknetRpcApiError> {
    let block_header = block_info.header();

    // safe unwrap because address is always valid and static
    let fee_token_address = FeeTokenAddresses {
        strk_fee_token_address: STRK_TOKEN_ADDR.to_stark_felt().try_into().unwrap(),
        eth_fee_token_address: ETH_TOKEN_ADDR.to_stark_felt().try_into().unwrap(),
    };
    let chain_id = starknet_api::core::ChainId("SN_MAIN".to_string());

    block_header
        .into_block_context(fee_token_address, chain_id)
        .map_err(|e| StarknetRpcApiError::ErrUnexpectedError { data: e.to_string() })
}

#[derive(thiserror::Error, Debug)]
#[error("Executing tx {hash:#} (index {index}): {err:#}")]
pub struct TransactionsExecError {
    hash: TransactionHash,
    index: usize,
    err: TransactionExecutionError,
}

pub struct ExecutionResult {
    pub hash: TransactionHash,
    pub tx_type: TxType,
    pub execution_info: TransactionExecutionInfo,
    pub state_diff: CommitmentStateDiff,
}

pub fn execute_transactions(
    starknet: &Starknet,
    transactions_before: impl IntoIterator<Item = Transaction>,
    transactions_to_trace: impl IntoIterator<Item = Transaction>,
    block_context: &BlockContext,
) -> Result<Vec<ExecutionResult>, TransactionsExecError> {
    let charge_fee = block_context.block_info().gas_prices.eth_l1_gas_price.get() != 0;
    let mut cached_state = init_cached_state(starknet, block_context);

    let mut executed_prev = 0;
    for (index, tx) in transactions_before.into_iter().enumerate() {
        let hash = tx.hash();
        log::debug!("executing {hash:?}");
        tx.execute(&mut cached_state, block_context, charge_fee, true).map_err(|err| TransactionsExecError {
            hash,
            index,
            err,
        })?;
        executed_prev += 1;
    }

    transactions_to_trace
        .into_iter()
        .enumerate()
        .map(|(index, tx)| {
            let hash = tx.hash();
            log::debug!("executing {hash:?} (2)");
            let tx_type = (&tx).into();
            let mut state = CachedState::<_>::create_transactional(&mut cached_state);
            let execution_info = tx
                .execute(&mut state, block_context, charge_fee, true)
                .map_err(|err| TransactionsExecError { hash, index: executed_prev + index, err })?;
            let state_diff = state.to_state_diff();
            state.commit();
            Ok(ExecutionResult { hash, tx_type, execution_info, state_diff })
        })
        .collect::<Result<Vec<_>, _>>()
}

trait Hash {
    fn hash(&self) -> TransactionHash;
}

impl Hash for Transaction {
    fn hash(&self) -> TransactionHash {
        match self {
            Self::AccountTransaction(tx) => tx.hash(),
            Self::L1HandlerTransaction(tx) => tx.tx_hash,
        }
    }
}

impl Hash for AccountTransaction {
    fn hash(&self) -> TransactionHash {
        match self {
            Self::Declare(tx) => tx.tx_hash,
            Self::DeployAccount(tx) => tx.tx_hash,
            Self::Invoke(tx) => tx.tx_hash,
        }
    }
}

pub fn simulate_transactions(
    starknet: &Starknet,
    transactions: Vec<AccountTransaction>,
    simulation_flags: &SimulationFlags,
    block_context: &BlockContext,
) -> Result<Vec<ExecutionResult>, TransactionExecutionError> {
    let mut cached_state = init_cached_state(starknet, block_context);

    transactions
        .into_iter()
        .map(|tx| {
            let hash = tx.hash();
            let tx_type = (&tx).into();
            let mut state = CachedState::<_>::create_transactional(&mut cached_state);
            let execution_info =
                tx.execute(&mut state, block_context, simulation_flags.charge_fee, simulation_flags.validate)?;
            let state_diff = state.to_state_diff();
            state.commit();
            Ok(ExecutionResult { hash, tx_type, execution_info, state_diff })
        })
        .collect::<Result<Vec<_>, _>>()
}

/// Call a smart contract function.
pub fn call_contract(
    starknet: &Starknet,
    address: ContractAddress,
    function_selector: EntryPointSelector,
    calldata: Calldata,
    block_context: &BlockContext,
) -> RpcResult<Vec<Felt>> {
    // Get class hash
    let class_hash = starknet
        .backend
        .contract_class_hash()
        .get(&address.to_felt())
        .or_internal_server_error("Error getting contract class hash")?
        .map(|felt| ClassHash(felt.to_stark_felt()));

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
            &mut BlockifierStateAdapter::new(
                Arc::clone(&starknet.backend),
                Some(block_context.block_info().block_number.0),
            ),
            &mut resources,
            &mut entry_point_execution_context,
        )
        .map_err(|err| {
            log::error!("Entry point execution error: {err}");
            StarknetRpcApiError::TxnExecutionError
        })?;

    log::debug!("Successfully called a smart contract function: {:?}", res);
    let result = res.execution.retdata.0.iter().map(|x| x.to_felt()).collect();
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
        .map(|tx| execute_fee_transaction(starknet, tx, validate, block_context))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(fees)
}

pub fn estimate_message_fee(
    starknet: &Starknet,
    message: L1HandlerTransaction,
    block_context: &BlockContext,
) -> Result<FeeEstimate, TransactionExecutionError> {
    let mut cached_state = init_cached_state(starknet, block_context);

    let unit = match message.fee_type() {
        blockifier::transaction::objects::FeeType::Strk => PriceUnit::Fri,
        blockifier::transaction::objects::FeeType::Eth => PriceUnit::Wei,
    };

    let tx_execution_infos = message.execute(&mut cached_state, block_context, true, true)?;

    // TODO: implement this
    // if !tx_execution_infos.is_reverted() {}

    let fee = FeeEstimate {
        gas_consumed: Felt::from(
            tx_execution_infos.actual_resources.0.get("l1_gas_usage").cloned().unwrap_or_default(),
        ),
        gas_price: tx_execution_infos.da_gas.l1_gas.into(),
        data_gas_consumed: tx_execution_infos.da_gas.l1_data_gas.into(),
        data_gas_price: tx_execution_infos.da_gas.l1_data_gas.into(),
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
    let prev_block = block_number.checked_sub(1); // handle genesis correctly
    CachedState::new(
        BlockifierStateAdapter::new(Arc::clone(&starknet.backend), prev_block),
        GlobalContractCache::new(16),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eth_token_addr() {
        let eth_token_addr =
            Felt::from_hex("0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7").unwrap();
        assert_eq!(ETH_TOKEN_ADDR, eth_token_addr);
    }

    #[test]
    fn test_strk_token_addr() {
        let strk_token_addr =
            Felt::from_hex("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d").unwrap();
        assert_eq!(STRK_TOKEN_ADDR, strk_token_addr);
    }
}
