use crate::{Error, ExecutionContext, ExecutionResult, TxExecError};
use blockifier::fee::fee_utils::get_fee_by_gas_vector;
use blockifier::fee::gas_usage::estimate_minimal_gas_vector;
use blockifier::state::cached_state::TransactionalState;
use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::errors::TransactionExecutionError;
use blockifier::transaction::objects::{HasRelatedFeeType, TransactionExecutionInfo};
use blockifier::transaction::transaction_execution::Transaction;
use blockifier::transaction::transactions::ExecutableTransaction;
use mp_convert::ToFelt;
use starknet_api::block::FeeType;
use starknet_api::contract_class::ContractClass;
use starknet_api::core::{ClassHash, ContractAddress, Nonce};
use starknet_api::executable_transaction::{AccountTransaction as ApiAccountTransaction, TransactionType};
use starknet_api::transaction::fields::{GasVectorComputationMode, Tip};
use starknet_api::transaction::{TransactionHash, TransactionVersion};

impl ExecutionContext {
    /// Execute transactions. The returned `ExecutionResult`s are the results of the `transactions_to_trace`. The results of `transactions_before` are discarded.
    /// This function is useful for tracing trasaction execution, by reexecuting the block.
    pub fn re_execute_transactions(
        &self,
        transactions_before: impl IntoIterator<Item = Transaction>,
        transactions_to_trace: impl IntoIterator<Item = Transaction>,
    ) -> Result<Vec<ExecutionResult>, Error> {
        let mut cached_state = self.init_cached_state();

        let mut executed_prev = 0;
        for (index, tx) in transactions_before.into_iter().enumerate() {
            let hash = tx.tx_hash();
            tracing::debug!("executing {:#x}", hash.to_felt());
            tx.execute(&mut cached_state, &self.block_context).map_err(|err| TxExecError {
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
                let tip = match &tx {
                    // Accessing tip may panic if the transaction is not version 3, so we check the version explicitly.
                    Transaction::Account(tx) if tx.version() == TransactionVersion::THREE => tx.tip(),
                    _ => Tip::ZERO,
                };

                // We need to estimate gas too.
                let minimal_gas = match &tx {
                    Transaction::Account(tx) => {
                        Some(estimate_minimal_gas_vector(&self.block_context, tx, &GasVectorComputationMode::All))
                    }
                    Transaction::L1Handler(_) => None, // There is no minimal_l1_gas field for L1 handler transactions.
                };

                let make_reexec_error = |err| TxExecError {
                    block_n: self.latest_visible_block.into(),
                    hash,
                    index: executed_prev + index,
                    err,
                };

                let mut transactional_state = TransactionalState::create_transactional(&mut cached_state);
                // NB: We use execute_raw because execute already does transaactional state.
                let execution_info = tx
                    .execute_raw(&mut transactional_state, &self.block_context, false)
                    .map(|mut tx_info: TransactionExecutionInfo| {
                        // TODO: why was this here again?
                        if tx_info.receipt.fee.0 == 0 {
                            let gas_vector = tx_info.receipt.resources.to_gas_vector(
                                self.block_context.versioned_constants(),
                                self.block_context.block_info().use_kzg_da,
                                &GasVectorComputationMode::NoL2Gas,
                            );
                            // TODO
                            let real_fees =
                                get_fee_by_gas_vector(self.block_context.block_info(), gas_vector, &fee_type, tip);

                            tx_info.receipt.fee = real_fees;
                        }
                        tx_info
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
                    minimal_l1_gas: minimal_gas,
                    execution_info,
                    state_diff: state_diff.state_maps.into(),
                })
            })
            .collect::<Result<Vec<_>, _>>()
    }
}

pub trait TxInfo {
    fn contract_address(&self) -> ContractAddress;
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
        Self::tx_hash(self)
    }

    // FIXME: fix this, this is wrong for L1HandlerTxs.
    fn nonce(&self) -> Nonce {
        Self::nonce(self)
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
}
