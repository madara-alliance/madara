use crate::Transaction;
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
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
    pub fn checked_add(self, duration: Duration) -> Option<Self> {
        self.0.checked_add(duration.as_millis().try_into().ok()?).map(Self)
    }
}

/// A transaction that has been validated, but not yet included into a block.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidatedMempoolTx {
    pub tx: Transaction,
    /// Only filled in for L1HandlerTransaction.
    pub paid_fee_on_l1: Option<u128>,
    /// This field is always filled in with the sender_address, but it is the deployed
    /// contract address in case of DeployAccount. For L1HandlerTransactions this is the
    /// contract address receiving the l1 message.
    pub contract_address: Felt,
    /// Timestamp.
    pub arrived_at: TxTimestamp,
    /// Compiled class in case of a Declare transaction.
    pub converted_class: Option<ConvertedClass>,
    /// Computed transaction hash.
    pub tx_hash: Felt,
}

impl ValidatedMempoolTx {
    pub fn from_starknet_api(
        tx: ApiAccountTransaction,
        arrived_at: TxTimestamp,
        converted_class: Option<ConvertedClass>,
    ) -> Self {
        Self {
            contract_address: tx.contract_address().to_felt(),
            tx_hash: tx.tx_hash().to_felt(),
            tx: tx.into(),
            paid_fee_on_l1: None,
            arrived_at,
            converted_class,
        }
    }

    pub fn into_blockifier_for_sequencing(
        self,
    ) -> Result<(BTransaction, TxTimestamp, Option<ConvertedClass>), ValidatedToBlockifierTxError> {
        let tx_hash = TransactionHash(self.tx_hash);
        let tx = match self.tx {
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
                let converted_class =
                    self.converted_class.as_ref().ok_or(ValidatedToBlockifierTxError::MissingField("class_info"))?;
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
        Ok((tx, self.arrived_at, self.converted_class))
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
