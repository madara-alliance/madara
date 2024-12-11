use std::time::{Duration, SystemTime};

use blockifier::transaction::account_transaction::ExecutionFlags as AccountExecutionFlags;
use blockifier::transaction::{
    account_transaction::AccountTransaction as BAccountTransaction, errors::TransactionExecutionError,
    transaction_execution::Transaction as BTransaction,
};
use mc_db::mempool_db::SavedTransaction;
use mp_class::{compile::ClassCompilationError, ConvertedClass};
use starknet_api::executable_transaction::{
    AccountTransaction as ApiAccountTransaction, DeployAccountTransaction, InvokeTransaction,
};
use starknet_api::{
    contract_class::ClassInfo as ApiClassInfo,
    executable_transaction::{DeclareTransaction, L1HandlerTransaction},
};
use starknet_api::{
    core::ContractAddress,
    transaction::{fields::Fee, Transaction as StarknetApiTransaction, TransactionHash},
};
use starknet_types_core::felt::Felt;

pub fn blockifier_to_saved_tx(tx: &BTransaction, arrived_at: SystemTime) -> SavedTransaction {
    let arrived_at = arrived_at.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_millis();
    match tx {
        BTransaction::Account(BAccountTransaction {
            tx,
            execution_flags: AccountExecutionFlags { only_query, .. },
        }) => SavedTransaction {
            only_query: *only_query,
            tx: tx.clone().into(),
            paid_fee_on_l1: None,
            contract_address: None,
            arrived_at,
        },
        BTransaction::L1Handler(tx) => SavedTransaction {
            only_query: false,
            tx: StarknetApiTransaction::L1Handler(tx.tx.clone()).into(),
            paid_fee_on_l1: Some(tx.paid_fee_on_l1.0),
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
}

pub fn saved_to_blockifier_tx(
    saved_tx: SavedTransaction,
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

            let class_info: ApiClassInfo = converted_class.into();

            let tx = tx.try_into().map_err(|_| SavedToBlockifierTxError::InvalidContractAddress)?;

            let declare_tx: ApiAccountTransaction =
                ApiAccountTransaction::Declare(DeclareTransaction { tx, tx_hash, class_info });
            BTransaction::Account(BAccountTransaction {
                tx: declare_tx,
                execution_flags: AccountExecutionFlags::from((saved_tx.only_query, true, true)),
            })
        }
        mp_transactions::Transaction::DeployAccount(tx) => BTransaction::Account(BAccountTransaction {
            tx: ApiAccountTransaction::DeployAccount(DeployAccountTransaction {
                tx: tx.try_into().map_err(|_| SavedToBlockifierTxError::InvalidContractAddress)?,
                tx_hash,
                contract_address: ContractAddress::try_from(
                    saved_tx.contract_address.ok_or(SavedToBlockifierTxError::MissingField("contract_address"))?,
                )
                .map_err(|_| SavedToBlockifierTxError::InvalidContractAddress)?,
            }),
            execution_flags: AccountExecutionFlags::from((saved_tx.only_query, true, true)),
        }),
        mp_transactions::Transaction::Invoke(tx) => BTransaction::Account(BAccountTransaction {
            tx: ApiAccountTransaction::Invoke(InvokeTransaction {
                tx: tx.try_into().map_err(|_| SavedToBlockifierTxError::InvalidContractAddress)?,
                tx_hash,
            }),
            execution_flags: AccountExecutionFlags::from((saved_tx.only_query, true, true)),
        }),
        mp_transactions::Transaction::Deploy(_) => return Err(SavedToBlockifierTxError::DeployNotSupported),
    };

    Ok((tx, arrived_at))
}
