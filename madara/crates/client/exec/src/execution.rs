use crate::{Error, ExecutionContext, ExecutionResult, TxExecError};
use blockifier::fee::gas_usage::estimate_minimal_gas_vector;
use blockifier::state::cached_state::TransactionalState;
use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::errors::TransactionExecutionError;
use blockifier::transaction::objects::HasRelatedFeeType;
use blockifier::transaction::transaction_execution::Transaction;
use blockifier::transaction::transactions::ExecutableTransaction;
use mc_db::MadaraStorageRead;
use mp_convert::ToFelt;
use starknet_api::block::FeeType;
use starknet_api::contract_class::ContractClass;
use starknet_api::core::{ClassHash, ContractAddress, Nonce};
use starknet_api::executable_transaction::{AccountTransaction as ApiAccountTransaction, TransactionType};
use starknet_api::transaction::fields::{GasVectorComputationMode, Tip};
use starknet_api::transaction::{TransactionHash, TransactionVersion};

impl<D: MadaraStorageRead> ExecutionContext<D> {
    /// Execute transactions. The returned `ExecutionResult`s are the results of the `transactions_to_trace`. The results of `transactions_before` are discarded.
    /// This function is useful for tracing trasaction execution, by reexecuting the block.
    pub fn execute_transactions(
        &mut self,
        transactions_before: impl IntoIterator<Item = Transaction>,
        transactions_to_trace: impl IntoIterator<Item = Transaction>,
    ) -> Result<Vec<ExecutionResult>, Error> {
        let mut executed_prev = 0;
        for (index, tx) in transactions_before.into_iter().enumerate() {
            let hash = tx.tx_hash();
            tracing::debug!("executing {:#x}", hash.to_felt());
            tx.execute(&mut self.state, &self.block_context).map_err(|err| TxExecError {
                view: format!("{}", self.view()),
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
                    Transaction::Account(tx) => {
                        Some(estimate_minimal_gas_vector(&self.block_context, tx, &GasVectorComputationMode::All))
                    }
                    Transaction::L1Handler(_) => None, // There is no minimal_l1_gas field for L1 handler transactions.
                };

                let view = self.view().clone();
                let make_reexec_error =
                    |err| TxExecError { view: format!("{view}"), hash, index: executed_prev + index, err };

                let mut transactional_state = TransactionalState::create_transactional(&mut self.state);
                // NB: We use execute_raw because execute already does transaactional state.
                let execution_info =
                    tx.execute_raw(&mut transactional_state, &self.block_context, false).map_err(make_reexec_error)?;

                let state_diff = transactional_state
                    .to_state_diff()
                    .map_err(TransactionExecutionError::StateError)
                    .map_err(make_reexec_error)?;
                transactional_state.commit();

                let gas_vector_computation_mode = match tx {
                    Transaction::Account(tx) => tx.tx.resource_bounds().get_gas_vector_computation_mode(),
                    Transaction::L1Handler(_) => GasVectorComputationMode::NoL2Gas,
                };

                Ok(ExecutionResult {
                    hash,
                    tx_type,
                    fee_type,
                    minimal_l1_gas,
                    execution_info,
                    gas_vector_computation_mode,
                    state_diff: state_diff.state_maps.into(),
                })
            })
            .collect::<Result<Vec<_>, _>>()
    }
}

pub trait TxInfo {
    fn contract_address(&self) -> ContractAddress;
    fn tx_hash(&self) -> TransactionHash;
    fn tx_nonce(&self) -> Option<Nonce>;
    fn tx_type(&self) -> TransactionType;
    fn fee_type(&self) -> FeeType;
    fn tip(&self) -> Option<Tip>;
    fn is_only_query(&self) -> bool;
    fn deployed_contract_address(&self) -> Option<ContractAddress>;
    fn declared_class_hash(&self) -> Option<ClassHash>;
    fn declared_contract_class(&self) -> Option<(ClassHash, ContractClass)>;
    fn l1_handler_tx_nonce(&self) -> Option<Nonce>;
}

impl TxInfo for Transaction {
    fn tx_hash(&self) -> TransactionHash {
        Self::tx_hash(self)
    }

    fn tx_nonce(&self) -> Option<Nonce> {
        if self.tx_type() == TransactionType::L1Handler {
            None
        } else {
            Some(Self::nonce(self))
        }
    }

    fn contract_address(&self) -> ContractAddress {
        Self::sender_address(self)
    }

    fn is_only_query(&self) -> bool {
        match self {
            Self::Account(tx) => tx.execution_flags.only_query,
            Self::L1Handler(_) => false,
        }
    }

    fn tx_type(&self) -> TransactionType {
        match self {
            Self::Account(tx) => tx.tx_type(),
            Self::L1Handler(_) => TransactionType::L1Handler,
        }
    }

    fn tip(&self) -> Option<Tip> {
        match self {
            // function tip() is only available for Account transactions with version 3 otherwise it panics.
            Self::Account(tx) if tx.version() >= TransactionVersion::THREE => Some(tx.tip()),
            _ => None,
        }
    }

    fn fee_type(&self) -> FeeType {
        match self {
            Self::Account(tx) => tx.fee_type(),
            Self::L1Handler(tx) => tx.fee_type(),
        }
    }

    fn declared_class_hash(&self) -> Option<ClassHash> {
        match self {
            Self::Account(tx) => tx.class_hash(),
            _ => None,
        }
    }

    fn declared_contract_class(&self) -> Option<(ClassHash, ContractClass)> {
        match self {
            Self::Account(AccountTransaction { tx: ApiAccountTransaction::Declare(tx), .. }) => {
                Some((tx.class_hash(), tx.class_info.contract_class.clone()))
            }
            _ => None,
        }
    }

    fn deployed_contract_address(&self) -> Option<ContractAddress> {
        match self {
            Self::Account(AccountTransaction { tx: ApiAccountTransaction::DeployAccount(tx), .. }) => {
                Some(tx.contract_address)
            }
            _ => None,
        }
    }

    fn l1_handler_tx_nonce(&self) -> Option<Nonce> {
        match self {
            Self::L1Handler(tx) => Some(tx.tx.nonce),
            _ => None,
        }
    }
}
