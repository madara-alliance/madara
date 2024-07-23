use crate::{to_starknet_api::TransactionApiError, Transaction, TransactionWithHash};
use blockifier::{execution::errors::ContractClassError, transaction::errors::TransactionExecutionError};
use dp_class::{to_blockifier_class, ClassInfo, ComputeClassHash, ContractClass, ConvertedClass, ToCompiledClass};
use starknet_api::transaction::TransactionHash;
use starknet_types_core::felt::Felt;

#[derive(thiserror::Error, Debug)]
pub enum BroadcastedToBlockifierError {
    #[error("Failed to compile contract class: {0}")]
    CompilationFailed(anyhow::Error),
    #[error("Failed to convert program: {0}")]
    ProgramError(#[from] cairo_vm::types::errors::program_errors::ProgramError),
    #[error("Failed to compute legacy class hash: {0}")]
    ComputeLegacyClassHashFailed(anyhow::Error),
    #[error("Failed to convert transaction to starkneti-api: {0}")]
    ConvertToTxApiError(#[from] TransactionApiError),
    #[error("Failed to convert transaction to blockifier: {0}")]
    ConvertTxBlockifierError(#[from] TransactionExecutionError),
    #[error("Failed to convert contract class: {0}")]
    ConvertContractClassError(#[from] ContractClassError),
}

pub fn broadcasted_to_blockifier(
    transaction: starknet_core::types::BroadcastedTransaction,
    chain_id: Felt,
    block_number: Option<u64>,
) -> Result<
    (blockifier::transaction::transaction_execution::Transaction, Option<ConvertedClass>),
    BroadcastedToBlockifierError,
> {
    let (class_info, class_hash, extra_class_info) = match &transaction {
        starknet_core::types::BroadcastedTransaction::Declare(tx) => match tx {
            starknet_core::types::BroadcastedDeclareTransaction::V1(tx) => {
                let compiled = tx.contract_class.compile().map_err(BroadcastedToBlockifierError::CompilationFailed)?;
                let compiled_class_hash = Felt::ZERO; // TODO(classes): check this is correct for legacy
                let class_hash = tx
                    .contract_class
                    .class_hash()
                    .map_err(BroadcastedToBlockifierError::ComputeLegacyClassHashFailed)?;
                let class_info = ClassInfo {
                    contract_class: ContractClass::Legacy((*tx.contract_class).clone().into()),
                    compiled_class_hash,
                    block_number,
                };

                (
                    Some(blockifier::execution::contract_class::ClassInfo::new(
                        &to_blockifier_class(compiled.clone())?,
                        0,
                        0,
                    )?),
                    Some(class_hash),
                    Some(ConvertedClass {
                        class_infos: (class_hash, class_info),
                        class_compiled: (compiled_class_hash, compiled),
                    }),
                )
            }
            starknet_core::types::BroadcastedDeclareTransaction::V2(tx) => {
                let compiled = tx.contract_class.compile().map_err(BroadcastedToBlockifierError::CompilationFailed)?;
                let compiled_class_hash = tx.compiled_class_hash;
                let class_hash = tx.contract_class.class_hash();
                let class_info = ClassInfo {
                    contract_class: ContractClass::Sierra((*tx.contract_class).clone().into()),
                    compiled_class_hash,
                    block_number,
                };

                (
                    Some(blockifier::execution::contract_class::ClassInfo::new(
                        &to_blockifier_class(compiled.clone())?,
                        tx.contract_class.sierra_program.len(),
                        tx.contract_class.abi.len(),
                    )?),
                    Some(class_hash),
                    Some(ConvertedClass {
                        class_infos: (class_hash, class_info),
                        class_compiled: (compiled_class_hash, compiled),
                    }),
                )
            }
            starknet_core::types::BroadcastedDeclareTransaction::V3(tx) => {
                let compiled = tx.contract_class.compile().map_err(BroadcastedToBlockifierError::CompilationFailed)?;
                let compiled_class_hash = tx.compiled_class_hash;
                let class_hash = tx.contract_class.class_hash();
                let class_info = ClassInfo {
                    contract_class: ContractClass::Sierra((*tx.contract_class).clone().into()),
                    compiled_class_hash,
                    block_number,
                };

                (
                    Some(blockifier::execution::contract_class::ClassInfo::new(
                        &to_blockifier_class(compiled.clone())?,
                        tx.contract_class.sierra_program.len(),
                        tx.contract_class.abi.len(),
                    )?),
                    Some(class_hash),
                    Some(ConvertedClass {
                        class_infos: (class_hash, class_info),
                        class_compiled: (compiled_class_hash, compiled),
                    }),
                )
            }
        },
        _ => (None, None, None),
    };

    let is_query = is_query(&transaction);
    let TransactionWithHash { transaction, hash } =
        TransactionWithHash::from_broadcasted(transaction, chain_id, class_hash);
    let deployed_address = match &transaction {
        Transaction::DeployAccount(tx) => Some(tx.calculate_contract_address()),
        _ => None,
    };
    let transaction: starknet_api::transaction::Transaction = (&transaction).try_into()?;

    Ok((
        blockifier::transaction::transaction_execution::Transaction::from_api(
            transaction,
            TransactionHash(hash),
            class_info,
            None,
            deployed_address.map(|address| address.try_into().unwrap()),
            is_query,
        )?,
        extra_class_info,
    ))
}

fn is_query(transaction: &starknet_core::types::BroadcastedTransaction) -> bool {
    match transaction {
        starknet_core::types::BroadcastedTransaction::Invoke(tx) => match tx {
            starknet_core::types::BroadcastedInvokeTransaction::V1(tx) => tx.is_query,
            starknet_core::types::BroadcastedInvokeTransaction::V3(tx) => tx.is_query,
        },
        starknet_core::types::BroadcastedTransaction::Declare(tx) => match tx {
            starknet_core::types::BroadcastedDeclareTransaction::V1(tx) => tx.is_query,
            starknet_core::types::BroadcastedDeclareTransaction::V2(tx) => tx.is_query,
            starknet_core::types::BroadcastedDeclareTransaction::V3(tx) => tx.is_query,
        },
        starknet_core::types::BroadcastedTransaction::DeployAccount(tx) => match tx {
            starknet_core::types::BroadcastedDeployAccountTransaction::V1(tx) => tx.is_query,
            starknet_core::types::BroadcastedDeployAccountTransaction::V3(tx) => tx.is_query,
        },
    }
}
