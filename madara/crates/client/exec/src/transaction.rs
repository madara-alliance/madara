use blockifier::transaction::transaction_execution as btx;
use mc_db::{MadaraBackend, MadaraStorageError};
use mp_block::BlockId;
use mp_class::compile::ClassCompilationError;
use mp_convert::ToFelt;
use mp_transactions::TransactionWithHash;
use starknet_api::transaction::TransactionHash;
use std::{borrow::Cow, sync::Arc};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Class not found")]
    ClassNotFound,
    #[error(transparent)]
    Storage(#[from] MadaraStorageError),
    #[error("Class compilation error: {0:#}")]
    ClassCompilationError(#[from] ClassCompilationError),
    #[error("{0}")]
    Internal(Cow<'static, str>),
}

/// Convert an starknet-api Transaction to a blockifier Transaction
///
/// **note:** this function does not support deploy transaction
/// because it is not supported by blockifier
pub fn to_blockifier_transaction(
    backend: Arc<MadaraBackend>,
    block_id: BlockId,
    transaction: mp_transactions::Transaction,
    tx_hash: &TransactionHash,
) -> Result<btx::Transaction, Error> {
    if transaction.as_deploy().is_some() {
        return Err(Error::Internal("Unsupported deploy transaction type".to_string().into()));
    }

    let class =
        if let Some(tx) = transaction.as_declare() {
            Some(backend.get_converted_class(&block_id, tx.class_hash())?.ok_or_else(|| {
                Error::Internal(format!("No class found for class_hash={:#x}", tx.class_hash()).into())
            })?)
        } else {
            None
        };

    TransactionWithHash::new(transaction, tx_hash.to_felt())
        .into_blockifier(class.as_ref())
        .map_err(|err| Error::Internal(format!("Error converting class to blockifier format: {err:#}").into()))
}
