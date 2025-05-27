use crate::{Error, ExecutionContext, ExecutionResult, TxExecError, TxFeeEstimationError};
use blockifier::execution::contract_class::ContractClass;
use blockifier::fee::fee_utils::get_fee_by_gas_vector;
use blockifier::fee::gas_usage::estimate_minimal_gas_vector;
use blockifier::state::cached_state::TransactionalState;
use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::errors::TransactionExecutionError;
use blockifier::transaction::objects::{FeeType, HasRelatedFeeType, TransactionExecutionInfo};
use blockifier::transaction::transaction_execution::Transaction;
use blockifier::transaction::transaction_types::TransactionType;
use blockifier::transaction::transactions::{
    DeclareTransaction, DeployAccountTransaction, ExecutableTransaction, ExecutionFlags, InvokeTransaction,
    L1HandlerTransaction,
};
use mp_convert::ToFelt;
use starknet_api::core::{ClassHash, ContractAddress, Nonce};
use starknet_api::transaction::TransactionHash;

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
            tracing::debug!("executing {:#x}", hash.to_felt());
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
                tracing::debug!("executing {:#x} (trace)", hash.to_felt());
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
    fn contract_address(&self) -> ContractAddress;
    fn clone_blockifier_transaction(&self) -> Self;
    fn tx_hash(&self) -> TransactionHash;
    fn nonce(&self) -> Nonce;
    fn tx_type(&self) -> TransactionType;
    fn fee_type(&self) -> FeeType;
    fn is_only_query(&self) -> bool;
    fn deployed_contract_address(&self) -> Option<ContractAddress>;
    fn declared_class_hash(&self) -> Option<ClassHash>;
    fn declared_contract_class(&self) -> Option<(ClassHash, ContractClass)>;
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

    // FIXME: fix this, this is wrong for L1HandlerTxs.
    fn nonce(&self) -> Nonce {
        match self {
            Transaction::AccountTransaction(tx) => match tx {
                AccountTransaction::Declare(tx) => tx.tx.nonce(),
                AccountTransaction::DeployAccount(tx) => tx.tx.nonce(),
                AccountTransaction::Invoke(tx) => tx.tx.nonce(),
            },
            Transaction::L1HandlerTransaction(tx) => tx.tx.nonce,
        }
    }

    fn contract_address(&self) -> ContractAddress {
        match self {
            Transaction::AccountTransaction(tx) => match tx {
                AccountTransaction::Declare(tx) => tx.tx.sender_address(),
                AccountTransaction::DeployAccount(tx) => tx.contract_address,
                AccountTransaction::Invoke(tx) => tx.tx.sender_address(),
            },
            Transaction::L1HandlerTransaction(tx) => tx.tx.contract_address,
        }
    }

    fn is_only_query(&self) -> bool {
        match self {
            Transaction::AccountTransaction(tx) => match tx {
                AccountTransaction::Declare(tx) => tx.only_query(),
                AccountTransaction::DeployAccount(tx) => tx.only_query,
                AccountTransaction::Invoke(tx) => tx.only_query,
            },
            Transaction::L1HandlerTransaction(_) => false,
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

    fn declared_class_hash(&self) -> Option<ClassHash> {
        match self {
            Self::AccountTransaction(AccountTransaction::Declare(tx)) => Some(tx.class_hash()),
            _ => None,
        }
    }

    fn declared_contract_class(&self) -> Option<(ClassHash, ContractClass)> {
        match self {
            Self::AccountTransaction(AccountTransaction::Declare(tx)) => Some((tx.class_hash(), tx.contract_class())),
            _ => None,
        }
    }

    fn deployed_contract_address(&self) -> Option<ContractAddress> {
        match self {
            Self::AccountTransaction(AccountTransaction::DeployAccount(tx)) => Some(tx.contract_address),
            _ => None,
        }
    }

    fn clone_blockifier_transaction(&self) -> Transaction {
        match self {
            Self::AccountTransaction(account_tx) => Self::AccountTransaction(match account_tx {
                AccountTransaction::Declare(tx) => AccountTransaction::Declare(match tx.only_query() {
                    true => DeclareTransaction::new_for_query(tx.tx.clone(), tx.tx_hash, tx.class_info.clone())
                        .expect("Making blockifier transaction for query"),
                    false => DeclareTransaction::new(tx.tx.clone(), tx.tx_hash, tx.class_info.clone())
                        .expect("Making blockifier transaction"),
                }),
                AccountTransaction::DeployAccount(tx) => AccountTransaction::DeployAccount(DeployAccountTransaction {
                    tx: tx.tx.clone(),
                    tx_hash: tx.tx_hash,
                    contract_address: tx.contract_address,
                    only_query: tx.only_query,
                }),
                AccountTransaction::Invoke(tx) => AccountTransaction::Invoke(InvokeTransaction {
                    tx: tx.tx.clone(),
                    tx_hash: tx.tx_hash,
                    only_query: tx.only_query,
                }),
            }),
            Self::L1HandlerTransaction(tx) => Self::L1HandlerTransaction(L1HandlerTransaction {
                tx: tx.tx.clone(),
                tx_hash: tx.tx_hash,
                paid_fee_on_l1: tx.paid_fee_on_l1,
            }),
        }
    }
}
