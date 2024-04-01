use alloc::vec::Vec;

use blockifier::context::BlockContext;
use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::errors::TransactionExecutionError;
use blockifier::transaction::objects::TransactionExecutionInfo;
use blockifier::transaction::transaction_execution::Transaction;
use blockifier::transaction::transactions::{ExecutableTransaction, L1HandlerTransaction};
use frame_support::storage;
use mp_simulations::{PlaceHolderErrorTypeForFailedStarknetExecution, SimulationFlags};
use mp_transactions::{UserOrL1HandlerTransaction, UserTransaction, user_or_l1_into_tx_vec};
use sp_core::Get;
use sp_runtime::DispatchError;

use crate::{Config, Error, Pallet};

impl<T: Config> Pallet<T> {
    pub fn estimate_fee(transactions: Vec<UserTransaction>) -> Result<Vec<(u128, u128)>, DispatchError> {
        storage::transactional::with_transaction(|| {
            storage::TransactionOutcome::Rollback(Result::<_, DispatchError>::Ok(Self::estimate_fee_inner(
                transactions,
            )))
        })
        .map_err(|_| Error::<T>::FailedToCreateATransactionalStorageExecution)?
    }

    fn estimate_fee_inner(transactions: Vec<UserTransaction>) -> Result<Vec<(u128, u128)>, DispatchError> {
        let transactions_len = transactions.len();
        let chain_id = Self::chain_id();
        let block_context = Self::get_block_context();

        let fee_res_iterator = transactions
            .into_iter()
            .map(|tx| {
                match Self::execute_account_transaction(tx.into(), &block_context, &SimulationFlags::default()) {
                    Ok(execution_info) if !execution_info.is_reverted() => Ok(execution_info),
                    Err(e) => {
                        log::error!("Transaction execution failed during fee estimation: {e}");
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
                }
            })
            .map(|exec_info_res| {
                exec_info_res.map(|exec_info| {
                    exec_info
                        .actual_resources
                        .0
                        .get("l1_gas_usage")
                        .ok_or_else(|| DispatchError::from(Error::<T>::MissingL1GasUsage))
                        .map(|l1_gas_usage| (exec_info.actual_fee.0, *l1_gas_usage))
                })
            });

        let mut fees = Vec::with_capacity(transactions_len);
        for fee_res in fee_res_iterator {
            fees.push(fee_res??);
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
    ) -> Result<Result<Vec<TransactionExecutionInfo>, PlaceHolderErrorTypeForFailedStarknetExecution>, DispatchError>
    {
        storage::transactional::with_transaction(|| {
            storage::TransactionOutcome::Rollback(Result::<_, DispatchError>::Ok(Self::re_execute_transactions_inner(
                transactions_before,
                transactions_to_trace,
            )))
        })
        .map_err(|_| Error::<T>::FailedToCreateATransactionalStorageExecution)?
    }

    fn re_execute_transactions_inner(
        transactions_before: Vec<UserOrL1HandlerTransaction>,
        transactions_to_trace: Vec<UserOrL1HandlerTransaction>,
    ) -> Result<Result<Vec<TransactionExecutionInfo>, PlaceHolderErrorTypeForFailedStarknetExecution>, DispatchError>
    {
        let block_context = Self::get_block_context();

        let _transactions_before_exec_infos = Self::execute_account_or_l1_handler_transactions(
            user_or_l1_into_tx_vec(transactions_before),
            &block_context,
            &SimulationFlags::default(),
        );
        let transactions_exec_infos = Self::execute_account_or_l1_handler_transactions(
            user_or_l1_into_tx_vec(transactions_to_trace),
            &block_context,
            &SimulationFlags::default(),
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
