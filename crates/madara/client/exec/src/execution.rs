use blockifier::fee::fee_utils::get_fee_by_gas_vector;
use blockifier::fee::gas_usage::estimate_minimal_gas_vector;
use blockifier::state::cached_state::TransactionalState;
use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::errors::TransactionExecutionError;
use blockifier::transaction::objects::{FeeType, HasRelatedFeeType, TransactionExecutionInfo};
use blockifier::transaction::transaction_execution::Transaction;
use blockifier::transaction::transaction_types::TransactionType;
use blockifier::transaction::transactions::{ExecutableTransaction, ExecutionFlags};
use starknet_api::transaction::TransactionHash;

use crate::{Error, ExecutionContext, ExecutionResult, TxExecError, TxFeeEstimationError};

impl ExecutionContext {
    /// Execute transactions. The returned `ExecutionResult`s are the results of the `transactions_to_trace`. The results of `transactions_before` are discarded.
    /// This function is useful for tracing trasaction execution, by reexecuting the block.
    pub fn re_execute_transactions(
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
            tracing::debug!("executing {hash:#}");
            tx.execute(&mut cached_state, &self.block_context, charge_fee, validate).map_err(|err| TxExecError {
                block_n: self.latest_visible_block.into(),
                hash,
                index,
                err,
            })?;
            executed_prev += 1;
        }

        transactions_to_trace
            .into_iter()
            .enumerate()
            .map(|(index, tx): (_, Transaction)| {
                let hash = tx.tx_hash();
                tracing::debug!("executing {hash:#} (trace)");
                let tx_type = tx.tx_type();
                let fee_type = tx.fee_type();

                // We need to estimate gas too.
                let minimal_l1_gas = match &tx {
                    Transaction::AccountTransaction(tx) => Some(
                        estimate_minimal_gas_vector(&self.block_context, tx)
                            .map_err(TransactionExecutionError::TransactionPreValidationError)
                            .map_err(|err| TxFeeEstimationError {
                                block_n: self.latest_visible_block.into(),
                                index,
                                err,
                            })?,
                    ),
                    Transaction::L1HandlerTransaction(_) => None, // There is no minimal_l1_gas field for L1 handler transactions.
                };

                let make_reexec_error = |err| TxExecError {
                    block_n: self.latest_visible_block.into(),
                    hash,
                    index: executed_prev + index,
                    err,
                };

                let mut transactional_state = TransactionalState::create_transactional(&mut cached_state);
                let execution_flags = ExecutionFlags { charge_fee, validate, concurrency_mode: false };
                // NB: We use execute_raw because execute already does transaactional state.
                let execution_info = tx
                    .execute_raw(&mut transactional_state, &self.block_context, execution_flags)
                    .and_then(|mut tx_info: TransactionExecutionInfo| {
                        // TODO: why was this here again?
                        if tx_info.transaction_receipt.fee.0 == 0 {
                            let gas_vector = tx_info.transaction_receipt.resources.to_gas_vector(
                                self.block_context.versioned_constants(),
                                self.block_context.block_info().use_kzg_da,
                            )?;
                            let real_fees =
                                get_fee_by_gas_vector(self.block_context.block_info(), gas_vector, &fee_type);

                            tx_info.transaction_receipt.fee = real_fees;
                        }
                        Ok(tx_info)
                    })
                    .map_err(make_reexec_error)?;

                let state_diff = transactional_state
                    .to_state_diff()
                    .map_err(TransactionExecutionError::StateError)
                    .map_err(make_reexec_error)?;
                transactional_state.commit();

                Ok(ExecutionResult {
                    hash,
                    tx_type,
                    fee_type,
                    minimal_l1_gas,
                    execution_info,
                    state_diff: state_diff.into(),
                })
            })
            .collect::<Result<Vec<_>, _>>()
    }
}

pub trait TxInfo {
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
