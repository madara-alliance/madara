use std::time::{Duration, SystemTime};

use blockifier::transaction::account_transaction::ExecutionFlags;
use blockifier::transaction::transaction_execution::Transaction as BTransaction;
use blockifier::transaction::{account_transaction::AccountTransaction, errors::TransactionExecutionError};
use mc_db::mempool_db::SerializedMempoolTx;
use mp_class::{compile::ClassCompilationError, ConvertedClass};
use mp_convert::ToFelt;
use starknet_api::executable_transaction::AccountTransaction as ApiAccountTransaction;
use starknet_api::executable_transaction::{
    DeclareTransaction, DeployAccountTransaction, InvokeTransaction, L1HandlerTransaction,
};
use starknet_api::transaction::{fields::Fee, Transaction as StarknetApiTransaction, TransactionHash};
use starknet_types_core::felt::Felt;

pub fn blockifier_to_saved_tx(
    tx: &blockifier::transaction::transaction_execution::Transaction,
    arrived_at: SystemTime,
) -> SerializedMempoolTx {
    let arrived_at = arrived_at.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_millis();
    match tx {
        BTransaction::Account(AccountTransaction { tx: ApiAccountTransaction::Declare(tx), execution_flags }) => {
            SerializedMempoolTx {
                only_query: execution_flags.only_query,
                tx: StarknetApiTransaction::Declare(tx.tx.clone()).into(),
                paid_fee_on_l1: None,
                contract_address: **tx.tx.sender_address(),
                arrived_at,
            }
        }
        BTransaction::Account(AccountTransaction { tx: ApiAccountTransaction::DeployAccount(tx), execution_flags }) => {
            SerializedMempoolTx {
                only_query: execution_flags.only_query,
                tx: StarknetApiTransaction::DeployAccount(tx.tx.clone()).into(),
                paid_fee_on_l1: None,
                contract_address: tx.contract_address.to_felt(),
                arrived_at,
            }
        }
        BTransaction::Account(AccountTransaction { tx: ApiAccountTransaction::Invoke(tx), execution_flags }) => {
            SerializedMempoolTx {
                only_query: execution_flags.only_query,
                tx: StarknetApiTransaction::Invoke(tx.tx.clone()).into(),
                paid_fee_on_l1: None,
                contract_address: **tx.tx.sender_address(),
                arrived_at,
            }
        }
        BTransaction::L1Handler(tx) => SerializedMempoolTx {
            only_query: false,
            tx: StarknetApiTransaction::L1Handler(tx.tx.clone()).into(),
            paid_fee_on_l1: Some(tx.paid_fee_on_l1.0),
            contract_address: **tx.tx.contract_address,
            arrived_at,
        },
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SavedToBlockifierTxError {
    #[error(transparent)]
    Blockifier(#[from] TransactionExecutionError),
    #[error("Missing field {0}")]
    MissingField(&'static str),
    #[error("Invalid contract address")]
    InvalidContractAddress,
    #[error("Deploy not supported")]
    DeployNotSupported,
    #[error("Converting class {0:#}")]
    ClassCompilationError(#[from] ClassCompilationError),
}

pub fn saved_to_blockifier_tx(
    saved_tx: SerializedMempoolTx,
    tx_hash: Felt,
    converted_class: &Option<ConvertedClass>,
) -> Result<(BTransaction, SystemTime), SavedToBlockifierTxError> {
    let tx_hash = TransactionHash(tx_hash);
    let arrived_at = SystemTime::UNIX_EPOCH + Duration::from_millis(saved_tx.arrived_at as u64);
    let tx = match saved_tx.tx {
        mp_transactions::Transaction::L1Handler(tx) => BTransaction::L1Handler(L1HandlerTransaction {
            tx: tx.try_into().map_err(|_| SavedToBlockifierTxError::InvalidContractAddress)?,
            tx_hash,
            paid_fee_on_l1: Fee(saved_tx
                .paid_fee_on_l1
                .ok_or(SavedToBlockifierTxError::MissingField("paid_fee_on_l1"))?),
        }),
        mp_transactions::Transaction::Declare(tx) => {
            let converted_class =
                converted_class.as_ref().ok_or(SavedToBlockifierTxError::MissingField("class_info"))?;

            let class_info = converted_class.into();
            let tx = tx.try_into().map_err(|_| SavedToBlockifierTxError::InvalidContractAddress)?;
            let declare_tx = DeclareTransaction { tx, tx_hash, class_info };
            BTransaction::Account(AccountTransaction {
                tx: ApiAccountTransaction::Declare(declare_tx),
                execution_flags: ExecutionFlags { only_query: saved_tx.only_query, charge_fee: true, validate: true },
            })
        }
        mp_transactions::Transaction::DeployAccount(tx) => BTransaction::Account(AccountTransaction {
            tx: ApiAccountTransaction::DeployAccount(DeployAccountTransaction {
                tx: tx.try_into().map_err(|_| SavedToBlockifierTxError::InvalidContractAddress)?,
                tx_hash,
                contract_address: saved_tx
                    .contract_address
                    .try_into()
                    .map_err(|_| SavedToBlockifierTxError::InvalidContractAddress)?,
            }),
            execution_flags: ExecutionFlags { only_query: saved_tx.only_query, charge_fee: true, validate: true },
        }),
        mp_transactions::Transaction::Invoke(tx) => BTransaction::Account(AccountTransaction {
            tx: ApiAccountTransaction::Invoke(InvokeTransaction {
                tx: tx.try_into().map_err(|_| SavedToBlockifierTxError::InvalidContractAddress)?,
                tx_hash,
            }),
            execution_flags: ExecutionFlags { only_query: saved_tx.only_query, charge_fee: true, validate: true },
        }),
        mp_transactions::Transaction::Deploy(_) => return Err(SavedToBlockifierTxError::DeployNotSupported),
    };

    Ok((tx, arrived_at))
}
