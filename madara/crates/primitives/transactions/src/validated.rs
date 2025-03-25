use crate::Transaction;
use blockifier::transaction::account_transaction::ExecutionFlags;
use blockifier::transaction::errors::TransactionExecutionError;
use blockifier::transaction::{
    account_transaction::AccountTransaction, transaction_execution::Transaction as BTransaction,
};
use mp_class::compile::ClassCompilationError;
use mp_class::ConvertedClass;
use mp_convert::{Felt, ToFelt};
use serde::{Deserialize, Serialize};
use starknet_api::executable_transaction::{
    AccountTransaction as ApiAccountTransaction, DeclareTransaction as ApiDeclareTransaction,
    DeployAccountTransaction as ApiDeployAccountTransaction, InvokeTransaction as ApiInvokeTransaction,
    L1HandlerTransaction as ApiL1HandlerTransaction,
};
use starknet_api::transaction::fields::Fee;
use starknet_api::transaction::{Transaction as ApiTransaction, TransactionHash};
use std::time::{Duration, SystemTime};

/// Timestamp, in millis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct TxTimestamp(pub u128);
impl TxTimestamp {
    pub const UNIX_EPOCH: Self = Self(0);

    pub fn now() -> Self {
        Self(SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_millis())
    }

    pub fn duration_since(self, other: TxTimestamp) -> Option<Duration> {
        self.0.checked_sub(other.0).map(|d| d.try_into().map(Duration::from_millis).unwrap_or(Duration::MAX))
    }

    pub fn checked_sub(self, duration: Duration) -> Option<Self> {
        self.0.checked_sub(duration.as_millis()).map(Self)
    }
    pub fn checked_add(self, duration: Duration) -> Option<Self> {
        self.0.checked_add(duration.as_millis()).map(Self)
    }
}

/// A transaction that has been validated, but not yet included into a block.
#[derive(Clone, Debug, Serialize, Deserialize)]
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
    pub fn from_blockifier(tx: BTransaction, arrived_at: TxTimestamp, converted_class: Option<ConvertedClass>) -> Self {
        match tx {
            BTransaction::Account(AccountTransaction { tx: ApiAccountTransaction::Declare(tx), .. }) => Self {
                contract_address: tx.tx.sender_address().to_felt(),
                tx: ApiTransaction::Declare(tx.tx).into(),
                paid_fee_on_l1: None,
                arrived_at,
                tx_hash: tx.tx_hash.to_felt(),
                converted_class,
            },
            BTransaction::Account(AccountTransaction { tx: ApiAccountTransaction::DeployAccount(tx), .. }) => Self {
                contract_address: tx.contract_address.to_felt(),
                tx: ApiTransaction::DeployAccount(tx.tx).into(),
                paid_fee_on_l1: None,
                arrived_at,
                tx_hash: tx.tx_hash.to_felt(),
                converted_class,
            },
            BTransaction::Account(AccountTransaction { tx: ApiAccountTransaction::Invoke(tx), .. }) => Self {
                contract_address: tx.tx.sender_address().to_felt(),
                tx: ApiTransaction::Invoke(tx.tx).into(),
                paid_fee_on_l1: None,
                arrived_at,
                tx_hash: tx.tx_hash.to_felt(),
                converted_class,
            },
            BTransaction::L1Handler(tx) => Self {
                contract_address: tx.tx.contract_address.to_felt(),
                tx: ApiTransaction::L1Handler(tx.tx).into(),
                paid_fee_on_l1: Some(tx.paid_fee_on_l1.0),
                arrived_at,
                tx_hash: tx.tx_hash.to_felt(),
                converted_class,
            },
        }
    }

    pub fn into_blockifier(
        self,
    ) -> Result<(BTransaction, TxTimestamp, Option<ConvertedClass>), ValidatedToBlockifierTxError> {
        let tx_hash = TransactionHash(self.tx_hash);
        let tx = match self.tx {
            Transaction::L1Handler(tx) => BTransaction::L1Handler(ApiL1HandlerTransaction {
                tx: tx.try_into().map_err(|_| ValidatedToBlockifierTxError::InvalidContractAddress)?,
                tx_hash,
                paid_fee_on_l1: Fee(self
                    .paid_fee_on_l1
                    .ok_or(ValidatedToBlockifierTxError::MissingField("paid_fee_on_l1"))?),
            }),
            Transaction::Declare(tx) => {
                let converted_class =
                    self.converted_class.as_ref().ok_or(ValidatedToBlockifierTxError::MissingField("class_info"))?;

                let class_info = converted_class.into();
                let tx = tx.try_into().map_err(|_| ValidatedToBlockifierTxError::InvalidContractAddress)?;
                let declare_tx = ApiDeclareTransaction { tx, tx_hash, class_info };
                BTransaction::Account(AccountTransaction {
                    tx: ApiAccountTransaction::Declare(declare_tx),
                    execution_flags: ExecutionFlags { only_query: false, charge_fee: true, validate: true },
                })
            }
            Transaction::DeployAccount(tx) => BTransaction::Account(AccountTransaction {
                tx: ApiAccountTransaction::DeployAccount(ApiDeployAccountTransaction {
                    tx: tx.try_into().map_err(|_| ValidatedToBlockifierTxError::InvalidContractAddress)?,
                    tx_hash,
                    contract_address: self
                        .contract_address
                        .try_into()
                        .map_err(|_| ValidatedToBlockifierTxError::InvalidContractAddress)?,
                }),
                execution_flags: ExecutionFlags { only_query: false, charge_fee: true, validate: true },
            }),
            Transaction::Invoke(tx) => BTransaction::Account(AccountTransaction {
                tx: ApiAccountTransaction::Invoke(ApiInvokeTransaction {
                    tx: tx.try_into().map_err(|_| ValidatedToBlockifierTxError::InvalidContractAddress)?,
                    tx_hash,
                }),
                execution_flags: ExecutionFlags { only_query: false, charge_fee: true, validate: true },
            }),
            Transaction::Deploy(_) => return Err(ValidatedToBlockifierTxError::DeployNotSupported),
        };

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
    ClassCompilationError(#[from] ClassCompilationError),
}
