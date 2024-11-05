use std::{borrow::Cow, sync::Arc};

use blockifier::execution::{contract_class::ClassInfo, errors::ContractClassError};
use blockifier::transaction::transaction_execution as btx;
use mc_db::{MadaraBackend, MadaraStorageError};
use mp_block::BlockId;
use mp_class::compile::ClassCompilationError;
use mp_convert::ToFelt;
use starknet_api::transaction::{Transaction, TransactionHash};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Class not found")]
    ClassNotFound,
    #[error(transparent)]
    Storage(#[from] MadaraStorageError),
    #[error("Class compilation error: {0:#}")]
    ClassCompilationError(#[from] ClassCompilationError),
    #[error("Contract class error: {0:#}")]
    ContractClassError(#[from] ContractClassError),
    #[error("{0}")]
    Internal(Cow<'static, str>),
    TransactionExecutionError(#[from] )
}

/// Convert an starknet-api Transaction to a blockifier Transaction
///
/// **note:** this function does not support deploy transaction
/// because it is not supported by blockifier
pub fn to_blockifier_transactions(
    backend: Arc<MadaraBackend>,
    block_id: BlockId,
    transaction: mp_transactions::Transaction,
    tx_hash: &TransactionHash,
) -> Result<btx::Transaction, Error> {
    let transaction: Transaction =
        transaction.try_into().map_err(|_| Error("Converting to starknet api transaction".into()))?;

    let paid_fee_on_l1 = match transaction {
        Transaction::L1Handler(_) => Some(starknet_api::transaction::Fee(1_000_000_000_000)),
        _ => None,
    };

    let class_info = match &transaction {
        Transaction::Declare(declare_tx) => {
            let class_hash = declare_tx.class_hash();
            let class_info = backend.get_class_info(&block_id, &class_hash.to_felt())?.ok_or(Error::ClassNotFound)?;

            match class_info {
                mp_class::ClassInfo::Sierra(info) => {
                    let compiled_class =
                        backend.get_sierra_compiled(&block_id, &info.compiled_class_hash)?.ok_or_else(|| {
                            Error::Internal(
                                "Inconsistent state: compiled sierra class from class_hash '{class_hash}' not found"
                                    .into(),
                            )
                        })?;

                    let blockifier_class = compiled_class.to_blockifier_class()?;
                    Some(ClassInfo::new(
                        &blockifier_class,
                        info.contract_class.program_length(),
                        info.contract_class.abi_length(),
                    )?)
                }
                mp_class::ClassInfo::Legacy(info) => {
                    let blockifier_class = info.contract_class.to_blockifier_class()?;
                    Some(ClassInfo::new(&blockifier_class, 0, 0)?)
                }
            }
        }
        _ => None,
    };

    btx::Transaction::from_api(transaction.clone(), *tx_hash, class_info, paid_fee_on_l1, None, false).map_err(|_| {
        tracing::error!("Failed to convert transaction to blockifier transaction");
        StarknetRpcApiError::InternalServerError
    })
}
