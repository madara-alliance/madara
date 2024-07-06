use blockifier::fee::gas_usage::estimate_minimal_gas_vector;
use blockifier::state::cached_state::CachedState;
use blockifier::state::state_api::State;
use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::errors::TransactionExecutionError;
use blockifier::transaction::objects::{FeeType, HasRelatedFeeType};
use blockifier::transaction::transaction_execution::Transaction;
use blockifier::transaction::transaction_types::TransactionType;
use blockifier::transaction::transactions::ExecutableTransaction;
use starknet_api::transaction::TransactionHash;

use crate::{Error, ExecutionContext, ExecutionResult, TxFeeEstimationError, TxReexecError};

impl<'a> ExecutionContext<'a> {
    pub fn execute_transactions(
        &self,
        transactions_before: impl IntoIterator<Item = Transaction>,
        transactions_to_trace: impl IntoIterator<Item = Transaction>,
        charge_fee: bool,
        validate: bool,
    ) -> Result<Vec<ExecutionResult>, Error> {
        let mut cached_state = self.init_cached_state();

        let mut executed_prev = 0;
        for (index, tx) in transactions_before.into_iter().enumerate() {
            let hash = tx.tx_hash();
            log::debug!("executing {hash:#}");
            tx.execute(&mut cached_state, &self.block_context, charge_fee, validate).map_err(|err| TxReexecError {
                block_n: self.db_id,
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
                let hash = tx.tx_hash();
                log::debug!("executing {hash:#} (trace)");
                let tx_type = tx.tx_type();
                let fee_type = tx.fee_type();

                let minimal_l1_gas = match &tx {
                    Transaction::AccountTransaction(tx) => Some(
                        estimate_minimal_gas_vector(&self.block_context, tx)
                            .map_err(TransactionExecutionError::TransactionPreValidationError)
                            .map_err(|err| TxFeeEstimationError { block_n: self.db_id, index, err })?,
                    ),
                    Transaction::L1HandlerTransaction(_) => None,
                };
                let mut state = CachedState::<_>::create_transactional(&mut cached_state);
                let execution_info = tx
                    .execute(&mut state, &self.block_context, charge_fee, validate)
                    .and_then(|mut tx_info| {
                        if tx_info.actual_fee.0 == 0 {
                            tx_info.actual_fee = blockifier::fee::fee_utils::calculate_tx_fee(
                                &tx_info.actual_resources,
                                &self.block_context,
                                &fee_type,
                            )?;
                        }
                        Ok(tx_info)
                    })
                    .map_err(|err| TxReexecError { block_n: self.db_id, hash, index: executed_prev + index, err })?;
                let state_diff = state.to_state_diff();
                state.commit();
                Ok(ExecutionResult { hash, tx_type, fee_type, minimal_l1_gas, execution_info, state_diff })
            })
            .collect::<Result<Vec<_>, _>>()
    }
}

trait TxInfo {
    fn tx_hash(&self) -> TransactionHash;
    fn tx_type(&self) -> TransactionType;
    fn fee_type(&self) -> FeeType;
}

impl TxInfo for Transaction {
    fn tx_hash(&self) -> TransactionHash {
        match self {
            Self::AccountTransaction(tx) => match tx {
                AccountTransaction::Declare(tx) => tx.tx_hash,
                AccountTransaction::DeployAccount(tx) => tx.tx_hash,
                AccountTransaction::Invoke(tx) => tx.tx_hash,
            },
            Self::L1HandlerTransaction(tx) => tx.tx_hash,
        }
    }

    fn tx_type(&self) -> TransactionType {
        match self {
            Self::AccountTransaction(tx) => tx.tx_type(),
            Self::L1HandlerTransaction(_) => TransactionType::L1Handler,
        }
    }

    fn fee_type(&self) -> FeeType {
        match self {
            Self::AccountTransaction(tx) => tx.fee_type(),
            Self::L1HandlerTransaction(tx) => tx.fee_type(),
        }
    }
}
