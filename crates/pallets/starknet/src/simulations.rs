use alloc::vec::Vec;

use blockifier::context::BlockContext;
use blockifier::execution::errors::EntryPointExecutionError;
use blockifier::fee::gas_usage::estimate_minimal_gas_vector;
use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::errors::TransactionExecutionError;
use blockifier::transaction::objects::{GasVector, HasRelatedFeeType, TransactionExecutionInfo};
use blockifier::transaction::transaction_execution::Transaction;
use blockifier::transaction::transactions::{ExecutableTransaction, L1HandlerTransaction};
use frame_support::storage;
use mp_felt::Felt252Wrapper;
use mp_simulations::{PlaceHolderErrorTypeForFailedStarknetExecution, SimulationFlagForEstimateFee, SimulationFlags};
use sp_core::Get;
use sp_runtime::DispatchError;
use starknet_api::core::{ContractAddress, EntryPointSelector};

// use starknet_core::types::PriceUnit;
use crate::types::{FeeEstimate, PriceUnit};
use crate::{Config, Error, Pallet};

impl<T: Config> Pallet<T> {
    pub fn estimate_fee(
        transactions: Vec<AccountTransaction>,
        simulation_flags: &[SimulationFlagForEstimateFee],
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
        transactions: Vec<AccountTransaction>,
        simulation_flags: &[SimulationFlagForEstimateFee],
    ) -> Result<Vec<FeeEstimate>, DispatchError> {
        let transactions_len = transactions.len();
        let block_context = Self::get_block_context();

        let mut fees = Vec::with_capacity(transactions_len);

        // TODO: the vector of flags should be for each transaction
        for tx in transactions {
            for flag in simulation_flags.iter() {
                match Self::execute_fee_transaction(tx.clone(), &block_context, flag.clone()) {
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
        transactions: Vec<AccountTransaction>,
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
        transactions: Vec<AccountTransaction>,
        simulation_flags: &SimulationFlags,
    ) -> Result<Vec<Result<TransactionExecutionInfo, PlaceHolderErrorTypeForFailedStarknetExecution>>, DispatchError>
    {
        let block_context = Self::get_block_context();

        let tx_execution_results = transactions
            .into_iter()
            .map(|tx| {
                tx.execute(
                    &mut Self::init_cached_state(),
                    &block_context,
                    simulation_flags.charge_fee,
                    simulation_flags.validate,
                )
                .map_err(|e| {
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

        let tx_execution_result = Self::execute_message(message, &block_context, simulation_flags).map_err(|e| {
            log::error!("Transaction execution failed during simulation: {e}");
            PlaceHolderErrorTypeForFailedStarknetExecution
        });

        Ok(tx_execution_result)
    }

    pub fn estimate_message_fee(message: L1HandlerTransaction) -> Result<FeeEstimate, DispatchError> {
        storage::transactional::with_transaction(|| {
            storage::TransactionOutcome::Rollback(Result::<_, DispatchError>::Ok(Self::estimate_message_fee_inner(
                message,
            )))
        })
        .map_err(|_| Error::<T>::FailedToCreateATransactionalStorageExecution)?
    }

    fn estimate_message_fee_inner(message: L1HandlerTransaction) -> Result<FeeEstimate, DispatchError> {
        let mut cached_state = Self::init_cached_state();

        let tx_execution_infos =
            match message.clone().execute(&mut cached_state, &Self::get_block_context(), true, true) {
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

        let unit = match message.fee_type() {
            blockifier::transaction::objects::FeeType::Strk => PriceUnit::Fri,
            blockifier::transaction::objects::FeeType::Eth => PriceUnit::Wei,
        };

        let fee = FeeEstimate {
            gas_consumed: Felt252Wrapper::from(*tx_execution_infos.actual_resources.0.get("l1_gas_usage").unwrap()),
            gas_price: Felt252Wrapper::from(T::L1GasPrices::get().eth_l1_gas_price.get()),
            data_gas_consumed: tx_execution_infos.da_gas.l1_data_gas.into(),
            data_gas_price: Felt252Wrapper::from(T::L1GasPrices::get().eth_l1_data_gas_price.get()),
            overall_fee: tx_execution_infos.actual_fee.0.into(),
            unit,
        };
        Ok(fee)
    }

    pub fn re_execute_transactions(
        transactions_before: Vec<Transaction>,
        transactions_to_trace: Vec<Transaction>,
        block_context: &BlockContext,
    ) -> Result<Vec<TransactionExecutionInfo>, PlaceHolderErrorTypeForFailedStarknetExecution> {
        let charge_fee = block_context.block_info().gas_prices.eth_l1_gas_price.get() != 1;
        let mut cached_state = Self::init_cached_state();

        transactions_before
            .into_iter()
            .map(|tx| tx.execute(&mut cached_state, block_context, charge_fee, false))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                log::error!("Transaction execution failed during re-execution: {e}");
                PlaceHolderErrorTypeForFailedStarknetExecution
            })?;

        let transactions_exec_infos = transactions_to_trace
            .into_iter()
            .map(|tx| tx.execute(&mut cached_state, block_context, charge_fee, false))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                log::error!("Transaction execution failed during re-execution: {e}");
                PlaceHolderErrorTypeForFailedStarknetExecution
            })?;

        Ok(transactions_exec_infos)
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
            .map_err(TransactionExecutionError::TransactionPreValidationError);

        let tx_info: Result<
            blockifier::transaction::objects::TransactionExecutionInfo,
            blockifier::transaction::errors::TransactionExecutionError,
        > = transaction.execute(&mut cached_state, block_context, false, simulation_flags.skip_validate).and_then(
            |mut tx_info| {
                if tx_info.actual_fee.0 == 0 {
                    tx_info.actual_fee = blockifier::fee::fee_utils::calculate_tx_fee(
                        &tx_info.actual_resources,
                        block_context,
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
            Err(_error) => Err(TransactionExecutionError::ExecutionError {
                error: EntryPointExecutionError::InternalError("Execution error".to_string()),
                storage_address: ContractAddress::default(),
                selector: EntryPointSelector::default(),
            }),
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
