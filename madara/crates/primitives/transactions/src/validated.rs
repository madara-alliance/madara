use crate::{Transaction, TransactionWithHash};
use blockifier::transaction::errors::TransactionExecutionError;
use blockifier::transaction::transaction_execution::Transaction as BTransaction;
use mp_class::ConvertedClass;
use mp_convert::{Felt, ToFelt};
use serde::{Deserialize, Serialize};
use starknet_api::executable_transaction::{
    AccountTransaction as ApiAccountTransaction, DeclareTransaction, DeployAccountTransaction, InvokeTransaction,
    L1HandlerTransaction,
};
use starknet_api::transaction::{fields::Fee, TransactionHash};
use std::time::{Duration, SystemTime};

/// Timestamp, in millis.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct TxTimestamp(pub u64);
impl TxTimestamp {
    /// 1970-01-01 00:00:00 UTC
    pub const UNIX_EPOCH: Self = Self(0);

    pub fn now() -> Self {
        Self(
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .try_into()
                .expect("Current time in millis overflows u64"),
        )
    }

    pub fn duration_since(self, other: TxTimestamp) -> Option<Duration> {
        self.0.checked_sub(other.0).map(Duration::from_millis)
    }

    pub fn checked_sub(self, duration: Duration) -> Option<Self> {
        self.0.checked_sub(duration.as_millis().try_into().ok()?).map(Self)
    }
    pub fn saturating_sub(self, duration: Duration) -> Self {
        Self(self.0.saturating_sub(duration.as_millis().try_into().unwrap_or(u64::MAX)))
    }
    pub fn checked_add(self, duration: Duration) -> Option<Self> {
        self.0.checked_add(duration.as_millis().try_into().ok()?).map(Self)
    }
    pub fn saturating_add(self, duration: Duration) -> Self {
        Self(self.0.saturating_add(duration.as_millis().try_into().unwrap_or(u64::MAX)))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionWithHashAndContractAddress {
    pub transaction: TransactionWithHash,
    /// This field is always filled in with the sender_address, but it is the deployed
    /// contract address in case of DeployAccount. For L1HandlerTransactions this is the
    /// contract address receiving the l1 message.
    pub contract_address: Felt,
}

/// A transaction that has been validated, but not yet included into a block.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidatedTransaction {
    pub transaction: Transaction,
    /// Only filled in for L1HandlerTransaction.
    pub paid_fee_on_l1: Option<u128>,
    /// This field is always filled in with the sender_address, but it is the deployed
    /// contract address in case of DeployAccount. For L1HandlerTransactions this is the
    /// contract address receiving the l1 message.
    // TODO: this field is redundant, it's already included in the transaction.
    pub contract_address: Felt,
    /// Timestamp.
    pub arrived_at: TxTimestamp,
    /// Compiled class in case of a Declare transaction.
    pub declared_class: Option<ConvertedClass>,
    /// Computed transaction hash.
    pub hash: Felt,
    /// Charge Fee Execution flag
    pub charge_fee: bool,
}

impl ValidatedTransaction {
    pub fn from_starknet_api(
        tx: ApiAccountTransaction,
        arrived_at: TxTimestamp,
        converted_class: Option<ConvertedClass>,
        charge_fee: bool,
    ) -> Self {
        Self {
            contract_address: tx.contract_address().to_felt(),
            hash: tx.tx_hash().to_felt(),
            transaction: tx.into(),
            paid_fee_on_l1: None,
            arrived_at,
            declared_class: converted_class,
            charge_fee,
        }
    }

    pub fn into_blockifier_for_sequencing(
        self,
    ) -> Result<(BTransaction, TxTimestamp, Option<ConvertedClass>), ValidatedToBlockifierTxError> {
        let tx_hash = TransactionHash(self.hash);
        let tx = match self.transaction {
            Transaction::L1Handler(tx) => {
                let tx = tx.try_into().map_err(|_| ValidatedToBlockifierTxError::InvalidContractAddress)?;
                let paid_fee_on_l1 =
                    Fee(self.paid_fee_on_l1.ok_or(ValidatedToBlockifierTxError::MissingField("paid_fee_on_l1"))?);

                starknet_api::executable_transaction::Transaction::L1Handler(L1HandlerTransaction {
                    tx,
                    tx_hash,
                    paid_fee_on_l1,
                })
            }
            Transaction::Declare(tx) => {
                let converted_class = self
                    .declared_class
                    .as_ref()
                    .ok_or(ValidatedToBlockifierTxError::MissingField("converted_class"))?;
                let class_info =
                    converted_class.try_into().map_err(ValidatedToBlockifierTxError::ClassCompilationError)?;
                let tx = tx.try_into().map_err(|_| ValidatedToBlockifierTxError::InvalidContractAddress)?;

                starknet_api::executable_transaction::Transaction::Account(ApiAccountTransaction::Declare(
                    DeclareTransaction { tx, tx_hash, class_info },
                ))
            }
            Transaction::DeployAccount(tx) => {
                let contract_address = self
                    .contract_address
                    .try_into()
                    .map_err(|_| ValidatedToBlockifierTxError::InvalidContractAddress)?;
                let tx = tx.try_into().map_err(|_| ValidatedToBlockifierTxError::InvalidContractAddress)?;

                starknet_api::executable_transaction::Transaction::Account(ApiAccountTransaction::DeployAccount(
                    DeployAccountTransaction { tx, tx_hash, contract_address },
                ))
            }
            Transaction::Invoke(tx) => {
                let tx = tx.try_into().map_err(|_| ValidatedToBlockifierTxError::InvalidContractAddress)?;

                starknet_api::executable_transaction::Transaction::Account(ApiAccountTransaction::Invoke(
                    InvokeTransaction { tx, tx_hash },
                ))
            }
            Transaction::Deploy(_) => return Err(ValidatedToBlockifierTxError::DeployNotSupported),
        };

        let tx = BTransaction::new_for_sequencing(tx);
        let tx = match tx {
            BTransaction::Account(mut txn) => {
                txn.execution_flags.charge_fee = self.charge_fee;
                BTransaction::Account(txn)
            }
            _ => tx,
        };
        Ok((tx, self.arrived_at, self.declared_class))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ValidatedToBlockifierTxError {
    #[error(transparent)]
    Blockifier(#[from] TransactionExecutionError),
    #[error("Missing field {0}")]
    MissingField(&'static str),
    #[error("Invalid contract address")]
    InvalidContractAddress,
    #[error("Deploy not supported")]
    DeployNotSupported,
    #[error("Converting class {0:#}")]
    ClassCompilationError(serde_json::Error),
}
