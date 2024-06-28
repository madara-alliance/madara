use std::sync::Arc;

use blockifier::context::BlockContext;
use blockifier::fee::gas_usage::estimate_minimal_gas_vector;
use blockifier::state::cached_state::{CachedState, GlobalContractCache};
use blockifier::state::state_api::State;
use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::errors::TransactionExecutionError;
use blockifier::transaction::objects::{FeeType, HasRelatedFeeType};
use blockifier::transaction::transaction_execution::Transaction;
use blockifier::transaction::transaction_types::TransactionType;
use blockifier::transaction::transactions::ExecutableTransaction;
use dc_db::DeoxysBackend;
use starknet_api::transaction::TransactionHash;

use crate::blockifier_state_adapter::BlockifierStateAdapter;
use crate::{ExecutionResult, TransactionsExecError};

pub fn execute_transactions(
    deoxys_backend: Arc<DeoxysBackend>,
    transactions_before: impl IntoIterator<Item = Transaction>,
    transactions_to_trace: impl IntoIterator<Item = Transaction>,
    block_context: &BlockContext,
    charge_fee: bool,
    validate: bool,
) -> Result<Vec<ExecutionResult>, TransactionsExecError> {
    let mut cached_state = init_cached_state(deoxys_backend, block_context);

    let mut executed_prev = 0;
    for (index, tx) in transactions_before.into_iter().enumerate() {
        let hash = tx.tx_hash();
        tx.execute(&mut cached_state, block_context, charge_fee, validate).map_err(|err| TransactionsExecError {
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
            let tx_type = tx.tx_type();
            let fee_type = tx.fee_type();
            let minimal_l1_gas = match &tx {
                Transaction::AccountTransaction(tx) => {
                    Some(estimate_minimal_gas_vector(block_context, tx).map_err(|e| TransactionsExecError {
                        hash,
                        index: executed_prev + index,
                        err: TransactionExecutionError::TransactionPreValidationError(e),
                    })?)
                }
                Transaction::L1HandlerTransaction(_) => None,
            };
            let mut state = CachedState::<_>::create_transactional(&mut cached_state);
            let execution_info = tx
                .execute(&mut state, block_context, charge_fee, validate)
                .map_err(|err| TransactionsExecError { hash, index: executed_prev + index, err })?;
            let state_diff = state.to_state_diff();
            state.commit();
            Ok(ExecutionResult { hash, tx_type, fee_type, minimal_l1_gas, execution_info, state_diff })
        })
        .collect::<Result<Vec<_>, _>>()
}

pub(crate) fn init_cached_state(
    deoxys_backend: Arc<DeoxysBackend>,
    block_context: &BlockContext,
) -> CachedState<BlockifierStateAdapter> {
    let block_number = block_context.block_info().block_number.0;
    let prev_block = block_number.checked_sub(1); // handle genesis correctly
    CachedState::new(BlockifierStateAdapter::new(deoxys_backend, prev_block), GlobalContractCache::new(16))
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
