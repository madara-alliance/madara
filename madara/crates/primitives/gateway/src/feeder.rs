use crate::transaction::Transaction;
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;

#[derive(Copy, Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TransactionStatus {
    #[default]
    NotReceived,
    Received,
    Pending,
    Rejected,
    AcceptedOnL1,
    AcceptedOnL2,
    Reverted,
    Aborted,
    Candidate,
    PreConfirmed,
}

#[derive(Copy, Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TransactionExecutionStatus {
    #[default]
    Succeeded,
    Reverted,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct TxFailureReason {
    pub code: String,
    pub error_message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProviderTransactionStatus {
    pub tx_status: TransactionStatus,
    pub finality_status: TransactionStatus,
    pub execution_status: Option<TransactionExecutionStatus>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_hash: Option<Felt>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_failure_reason: Option<TxFailureReason>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_revert_reason: Option<String>,
}

impl ProviderTransactionStatus {
    pub fn with_status(
        tx_status: TransactionStatus,
        execution_status: Option<TransactionExecutionStatus>,
        block_hash: Option<Felt>,
        tx_revert_reason: Option<String>,
    ) -> Self {
        Self {
            tx_status,
            finality_status: tx_status,
            execution_status,
            block_hash,
            tx_failure_reason: None,
            tx_revert_reason,
        }
    }

    pub fn not_received() -> Self {
        Self::with_status(TransactionStatus::NotReceived, None, None, None)
    }

    pub fn received() -> Self {
        Self::with_status(TransactionStatus::Received, None, None, None)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ProviderTransactionResponse {
    pub status: TransactionStatus,
    pub finality_status: TransactionStatus,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_status: Option<TransactionExecutionStatus>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_hash: Option<Felt>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_number: Option<u64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_index: Option<u64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction: Option<Transaction>,
}

impl ProviderTransactionResponse {
    pub fn with_status(
        status: TransactionStatus,
        execution_status: Option<TransactionExecutionStatus>,
        block_hash: Option<Felt>,
        block_number: Option<u64>,
        transaction_index: Option<u64>,
        transaction: Option<Transaction>,
    ) -> Self {
        Self {
            status,
            finality_status: status,
            execution_status,
            block_hash,
            block_number,
            transaction_index,
            transaction,
        }
    }

    pub fn not_received() -> Self {
        Self::with_status(TransactionStatus::NotReceived, None, None, None, None, None)
    }

    pub fn received() -> Self {
        Self::with_status(TransactionStatus::Received, None, None, None, None, None)
    }
}
