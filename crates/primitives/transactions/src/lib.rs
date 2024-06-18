pub mod compute_hash;
pub mod from_broadcasted_transactions;
pub mod getters;
pub mod to_starknet_core_transaction;
pub mod utils;

use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::transaction_execution::Transaction;
use blockifier::transaction::transaction_types::TransactionType;

use starknet_types_core::felt::Felt;

const SIMULATE_TX_VERSION_OFFSET: Felt =
    Felt::from_raw([576460752142434320, 18446744073709551584, 17407, 18446744073700081665]);

/// Legacy check for deprecated txs
/// See `https://docs.starknet.io/documentation/architecture_and_concepts/Blocks/transactions/` for more details.

pub const LEGACY_BLOCK_NUMBER: u64 = 1470;
pub const LEGACY_L1_HANDLER_BLOCK: u64 = 854;

/// Wrapper type for transaction execution error.
/// Different tx types.
/// See `https://docs.starknet.io/documentation/architecture_and_concepts/Blocks/transactions/` for more details.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TxType {
    /// Regular invoke transaction.
    Invoke,
    /// Declare transaction.
    Declare,
    /// Deploy account transaction.
    DeployAccount,
    /// Message sent from ethereum.
    L1Handler,
}

impl From<TxType> for TransactionType {
    fn from(value: TxType) -> Self {
        match value {
            TxType::Invoke => TransactionType::InvokeFunction,
            TxType::Declare => TransactionType::Declare,
            TxType::DeployAccount => TransactionType::DeployAccount,
            TxType::L1Handler => TransactionType::L1Handler,
        }
    }
}

impl From<&Transaction> for TxType {
    fn from(value: &Transaction) -> Self {
        match value {
            Transaction::AccountTransaction(tx) => tx.into(),
            Transaction::L1HandlerTransaction(_) => TxType::L1Handler,
        }
    }
}

impl From<&AccountTransaction> for TxType {
    fn from(value: &AccountTransaction) -> Self {
        match value {
            AccountTransaction::Declare(_) => TxType::Declare,
            AccountTransaction::DeployAccount(_) => TxType::DeployAccount,
            AccountTransaction::Invoke(_) => TxType::Invoke,
        }
    }
}
