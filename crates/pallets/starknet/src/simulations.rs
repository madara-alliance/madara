use alloc::vec::Vec;

use blockifier::context::BlockContext;
use blockifier::execution::errors::EntryPointExecutionError;
use blockifier::fee;
use blockifier::fee::gas_usage::estimate_minimal_gas_vector;
use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::errors::{TransactionExecutionError, TransactionFeeError};
use blockifier::transaction::objects::{GasVector, HasRelatedFeeType, TransactionExecutionInfo};
use blockifier::transaction::transaction_execution::Transaction;
use blockifier::transaction::transactions::{ExecutableTransaction, L1HandlerTransaction};
use frame_support::storage;
use mp_simulations::{PlaceHolderErrorTypeForFailedStarknetExecution, SimulationFlagForEstimateFee, SimulationFlags};
use mp_transactions::{user_or_l1_into_tx_vec, UserOrL1HandlerTransaction, UserTransaction};
use sp_core::Get;
use sp_runtime::DispatchError;
use starknet_api::core::{ContractAddress, EntryPointSelector};
use starknet_core::types::{FeeEstimate, PriceUnit};

use crate::{Config, Error, Pallet};

impl<T: Config> Pallet<T> {
    pub fn estimate_fee(
        transactions: Vec<UserTransaction>,
        simulation_flags: &Vec<SimulationFlagForEstimateFee>,
    ) -> Result<Vec<FeeEstimate>, DispatchError> {
        storage::transactional::with_transaction(|| {
            storage::TransactionOutcome::Rollback(Result::<_, DispatchError>::Ok(Self::estimate_fee_inner(
                transactions,
                simulation_flags,
            )))
        })
        .map_err(|_| Error::<T>::FailedToCreateATransactionalStorageExecution)?
    }

    fn estimate_fee_inner(
        transactions: Vec<UserTransaction>,
        simulation_flags: &Vec<SimulationFlagForEstimateFee>,
    ) -> Result<Vec<FeeEstimate>, DispatchError> {
        let transactions_len = transactions.len();
        let chain_id = Self::chain_id();
        let block_context = Self::get_block_context();

        let mut fees = Vec::with_capacity(transactions_len);

        for tx in transactions {
            for flag in simulation_flags.iter() {
                match Self::execute_fee_transaction(tx.clone().into(), &block_context, flag.clone()) {
                    Ok(execution_info) => fees.push(execution_info),
                    Err(e) => {
                        log::error!("Transaction execution failed during fee estimation: {e}");
                        return Err(Error::<T>::TransactionExecutionFailed.into());
                    }
                }
            }
        }

        Ok(fees)
    }

    pub fn simulate_transactions(
        transactions: Vec<UserTransaction>,
        simulation_flags: &SimulationFlags,
    ) -> Result<Vec<Result<TransactionExecutionInfo, PlaceHolderErrorTypeForFailedStarknetExecution>>, DispatchError>
    {
        storage::transactional::with_transaction(|| {
            storage::TransactionOutcome::Rollback(Result::<_, DispatchError>::Ok(Self::simulate_transactions_inner(
                transactions,
                simulation_flags,
            )))
        })
        .map_err(|_| Error::<T>::FailedToCreateATransactionalStorageExecution)?
    }

    fn simulate_transactions_inner(
        transactions: Vec<UserTransaction>,
        simulation_flags: &SimulationFlags,
    ) -> Result<Vec<Result<TransactionExecutionInfo, PlaceHolderErrorTypeForFailedStarknetExecution>>, DispatchError>
    {
        let block_context = Self::get_block_context();

        let tx_execution_results = transactions
            .into_iter()
            .map(|tx| {
                Self::execute_account_transaction(tx.into(), &block_context, simulation_flags).map_err(|e| {
                    log::error!("Transaction execution failed during simulation: {e}");
                    PlaceHolderErrorTypeForFailedStarknetExecution
                })
            })
            .collect();

        Ok(tx_execution_results)
    }

    pub fn simulate_message(
        message: L1HandlerTransaction,
        simulation_flags: &SimulationFlags,
    ) -> Result<Result<TransactionExecutionInfo, PlaceHolderErrorTypeForFailedStarknetExecution>, DispatchError> {
        storage::transactional::with_transaction(|| {
            storage::TransactionOutcome::Rollback(Result::<_, DispatchError>::Ok(Self::simulate_message_inner(
                message,
                simulation_flags,
            )))
        })
        .map_err(|_| Error::<T>::FailedToCreateATransactionalStorageExecution)?
    }

    fn simulate_message_inner(
        message: L1HandlerTransaction,
        simulation_flags: &SimulationFlags,
    ) -> Result<Result<TransactionExecutionInfo, PlaceHolderErrorTypeForFailedStarknetExecution>, DispatchError> {
        let block_context = Self::get_block_context();

        let tx_execution_result = Self::execute_message(message, &block_context, &simulation_flags).map_err(|e| {
            log::error!("Transaction execution failed during simulation: {e}");
            PlaceHolderErrorTypeForFailedStarknetExecution
        });

        Ok(tx_execution_result)
    }

    pub fn estimate_message_fee(message: L1HandlerTransaction) -> Result<(u128, u128, u128), DispatchError> {
        storage::transactional::with_transaction(|| {
            storage::TransactionOutcome::Rollback(Result::<_, DispatchError>::Ok(Self::estimate_message_fee_inner(
                message,
            )))
        })
        .map_err(|_| Error::<T>::FailedToCreateATransactionalStorageExecution)?
    }

    fn estimate_message_fee_inner(message: L1HandlerTransaction) -> Result<(u128, u128, u128), DispatchError> {
        let mut cached_state = Self::init_cached_state();

        let tx_execution_infos = match message.execute(&mut cached_state, &Self::get_block_context(), true, true) {
            Ok(execution_info) if !execution_info.is_reverted() => Ok(execution_info),
            Err(e) => {
                log::error!(
                    "Transaction execution failed during fee estimation: {e} {:?}",
                    std::error::Error::source(&e)
                );
                Err(Error::<T>::TransactionExecutionFailed)
            }
            Ok(execution_info) => {
                log::error!(
                    "Transaction execution reverted during fee estimation: {}",
                    // Safe due to the `match` branch order
                    execution_info.revert_error.unwrap()
                );
                Err(Error::<T>::TransactionExecutionFailed)
            }
        }?;

        if let Some(l1_gas_usage) = tx_execution_infos.actual_resources.0.get("l1_gas_usage") {
            Ok((T::L1GasPrices::get().eth_l1_gas_price.into(), tx_execution_infos.actual_fee.0, *l1_gas_usage))
        } else {
            Err(Error::<T>::MissingL1GasUsage.into())
        }
    }

    pub fn re_execute_transactions(
        transactions_before: Vec<UserOrL1HandlerTransaction>,
        transactions_to_trace: Vec<UserOrL1HandlerTransaction>,
        block_context: &BlockContext,
    ) -> Result<Result<Vec<TransactionExecutionInfo>, PlaceHolderErrorTypeForFailedStarknetExecution>, DispatchError>
    {
        storage::transactional::with_transaction(|| {
            storage::TransactionOutcome::Rollback(Result::<_, DispatchError>::Ok(Self::re_execute_transactions_inner(
                transactions_before,
                transactions_to_trace,
                block_context,
            )))
        })
        .map_err(|_| Error::<T>::FailedToCreateATransactionalStorageExecution)?
    }

    fn re_execute_transactions_inner(
        transactions_before: Vec<UserOrL1HandlerTransaction>,
        transactions_to_trace: Vec<UserOrL1HandlerTransaction>,
        block_context: &BlockContext,
    ) -> Result<Result<Vec<TransactionExecutionInfo>, PlaceHolderErrorTypeForFailedStarknetExecution>, DispatchError>
    {
        // desactivate fee charging for the first blocks
        let simulation_flags = SimulationFlags {
            charge_fee: if block_context.block_info().gas_prices.eth_l1_gas_price.get() == 1 { false } else { true },
            validate: false,
        };

        let _transactions_before_exec_infos = Self::execute_account_or_l1_handler_transactions(
            user_or_l1_into_tx_vec(transactions_before),
            &block_context,
            &simulation_flags,
        );
        let transactions_exec_infos = Self::execute_account_or_l1_handler_transactions(
            user_or_l1_into_tx_vec(transactions_to_trace),
            &block_context,
            &simulation_flags,
        );

        Ok(transactions_exec_infos)
    }

    fn execute_account_transaction(
        transaction: AccountTransaction,
        block_context: &BlockContext,
        simulation_flags: &SimulationFlags,
    ) -> Result<TransactionExecutionInfo, TransactionExecutionError> {
        let mut cached_state = Self::init_cached_state();

        transaction.execute(&mut cached_state, block_context, simulation_flags.charge_fee, simulation_flags.validate)
    }

    fn execute_fee_transaction(
        transaction: AccountTransaction,
        block_context: &BlockContext,
        simulation_flags: SimulationFlagForEstimateFee,
    ) -> Result<FeeEstimate, TransactionExecutionError> {
        let mut cached_state = Self::init_cached_state();

        let fee_type = transaction.fee_type();

        let gas_price = block_context.block_info().gas_prices.get_gas_price_by_fee_type(&fee_type).get();
        let data_gas_price = block_context.block_info().gas_prices.get_data_gas_price_by_fee_type(&fee_type).get();
        let unit = match fee_type {
            blockifier::transaction::objects::FeeType::Strk => PriceUnit::Fri,
            blockifier::transaction::objects::FeeType::Eth => PriceUnit::Wei,
        };

        let minimal_l1_gas_amount_vector = estimate_minimal_gas_vector(block_context, &transaction)
            .map_err(|e| TransactionExecutionError::TransactionPreValidationError(e));

        let tx_info: Result<
            blockifier::transaction::objects::TransactionExecutionInfo,
            blockifier::transaction::errors::TransactionExecutionError,
        > = transaction.execute(&mut cached_state, &block_context, false, simulation_flags.skip_validate).and_then(
            |mut tx_info| {
                if tx_info.actual_fee.0 == 0 {
                    tx_info.actual_fee = blockifier::fee::fee_utils::calculate_tx_fee(
                        &tx_info.actual_resources,
                        &block_context,
                        &fee_type,
                    )?;
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
            Err(_error) => {
                return Err(TransactionExecutionError::ExecutionError {
                    error: EntryPointExecutionError::InternalError("Execution error".to_string()),
                    storage_address: ContractAddress::default(),
                    selector: EntryPointSelector::default(),
                });
            }
        }
    }

    fn execute_message(
        transaction: L1HandlerTransaction,
        block_context: &BlockContext,
        simulation_flags: &SimulationFlags,
    ) -> Result<TransactionExecutionInfo, TransactionExecutionError> {
        let mut cached_state = Self::init_cached_state();

        transaction.execute(&mut cached_state, block_context, simulation_flags.charge_fee, simulation_flags.validate)
    }

    fn execute_account_or_l1_handler_transactions(
        transactions: Vec<Transaction>,
        block_context: &BlockContext,
        simulation_flags: &SimulationFlags,
    ) -> Result<Vec<TransactionExecutionInfo>, PlaceHolderErrorTypeForFailedStarknetExecution> {
        let mut cached_state = Self::init_cached_state();

        transactions
            .into_iter()
            .map(|user_or_l1_tx| match user_or_l1_tx {
                Transaction::AccountTransaction(tx) => {
                    Self::execute_account_transaction(tx, block_context, simulation_flags)
                }
                Transaction::L1HandlerTransaction(tx) => Self::execute_message(tx, block_context, simulation_flags),
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| PlaceHolderErrorTypeForFailedStarknetExecution)
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
