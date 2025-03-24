use blockifier::{
    execution::{contract_class::ClassInfo as BClassInfo, errors::ContractClassError},
    transaction::{
        account_transaction::AccountTransaction,
        errors::TransactionExecutionError,
        transaction_execution::Transaction as BTransaction,
        transactions::{DeclareTransaction, DeployAccountTransaction, InvokeTransaction, L1HandlerTransaction},
    },
};
use mc_db::mempool_db::SavedTransaction;
use mp_class::{compile::ClassCompilationError, ConvertedClass};
use mp_convert::ToFelt;
use starknet_api::{
    core::ContractAddress,
    transaction::{Fee, Transaction as StarknetApiTransaction, TransactionHash},
};
use starknet_types_core::felt::Felt;
use std::time::{Duration, SystemTime};

pub fn blockifier_to_saved_tx(tx: &BTransaction, arrived_at: SystemTime) -> SavedTransaction {
    let arrived_at = arrived_at.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_millis();
    match tx {
        BTransaction::AccountTransaction(AccountTransaction::Declare(tx)) => SavedTransaction {
            only_query: tx.only_query(),
            tx: StarknetApiTransaction::Declare(tx.tx.clone()).into(),
            paid_fee_on_l1: None,
            contract_address: None,
            arrived_at,
        },
        BTransaction::AccountTransaction(AccountTransaction::DeployAccount(tx)) => SavedTransaction {
            only_query: tx.only_query,
            tx: StarknetApiTransaction::DeployAccount(tx.tx.clone()).into(),
            paid_fee_on_l1: None,
            contract_address: Some(tx.contract_address.to_felt()),
            arrived_at,
        },
        BTransaction::AccountTransaction(AccountTransaction::Invoke(tx)) => SavedTransaction {
            only_query: tx.only_query,
            tx: StarknetApiTransaction::Invoke(tx.tx.clone()).into(),
            paid_fee_on_l1: None,
            contract_address: None,
            arrived_at,
        },
        BTransaction::L1HandlerTransaction(tx) => SavedTransaction {
            only_query: false,
            tx: StarknetApiTransaction::L1Handler(tx.tx.clone()).into(),
            paid_fee_on_l1: Some(*tx.paid_fee_on_l1),
            contract_address: None,
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
    #[error("Converting class {0:#}")]
    ContractClassError(#[from] ContractClassError),
}

pub fn saved_to_blockifier_tx(
    saved_tx: SavedTransaction,
    tx_hash: Felt,
    converted_class: &Option<ConvertedClass>,
) -> Result<(BTransaction, SystemTime), SavedToBlockifierTxError> {
    let tx_hash = TransactionHash(tx_hash);
    let arrived_at = SystemTime::UNIX_EPOCH + Duration::from_millis(saved_tx.arrived_at as u64);
    let tx = match saved_tx.tx {
        mp_transactions::Transaction::L1Handler(tx) => BTransaction::L1HandlerTransaction(L1HandlerTransaction {
            tx: tx.try_into().map_err(|_| SavedToBlockifierTxError::InvalidContractAddress)?,
            tx_hash,
            paid_fee_on_l1: Fee(saved_tx
                .paid_fee_on_l1
                .ok_or(SavedToBlockifierTxError::MissingField("paid_fee_on_l1"))?),
        }),
        mp_transactions::Transaction::Declare(tx) => {
            let converted_class =
                converted_class.as_ref().ok_or(SavedToBlockifierTxError::MissingField("class_info"))?;

            let class_info = match converted_class {
                ConvertedClass::Legacy(class) => {
                    BClassInfo::new(&class.info.contract_class.to_blockifier_class()?, 0, 0)?
                }
                ConvertedClass::Sierra(class) => BClassInfo::new(
                    &class.compiled.to_blockifier_class()?,
                    class.info.contract_class.sierra_program.len(),
                    class.info.contract_class.abi.len(),
                )?,
            };
            let tx = tx.try_into().map_err(|_| SavedToBlockifierTxError::InvalidContractAddress)?;
            let declare_tx = match saved_tx.only_query {
                true => DeclareTransaction::new_for_query(tx, tx_hash, class_info)?,
                false => DeclareTransaction::new(tx, tx_hash, class_info)?,
            };
            BTransaction::AccountTransaction(AccountTransaction::Declare(declare_tx))
        }
        mp_transactions::Transaction::DeployAccount(tx) => {
            BTransaction::AccountTransaction(AccountTransaction::DeployAccount(DeployAccountTransaction {
                tx: tx.try_into().map_err(|_| SavedToBlockifierTxError::InvalidContractAddress)?,
                tx_hash,
                contract_address: ContractAddress::try_from(
                    saved_tx.contract_address.ok_or(SavedToBlockifierTxError::MissingField("contract_address"))?,
                )
                .map_err(|_| SavedToBlockifierTxError::InvalidContractAddress)?,
                only_query: saved_tx.only_query,
            }))
        }
        mp_transactions::Transaction::Invoke(tx) => {
            BTransaction::AccountTransaction(AccountTransaction::Invoke(InvokeTransaction {
                tx: tx.try_into().map_err(|_| SavedToBlockifierTxError::InvalidContractAddress)?,
                tx_hash,
                only_query: saved_tx.only_query,
            }))
        }
        mp_transactions::Transaction::Deploy(_) => return Err(SavedToBlockifierTxError::DeployNotSupported),
    };

    Ok((tx, arrived_at))
}
