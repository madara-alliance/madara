use std::sync::Arc;

use crate::{into_starknet_api::TransactionApiError, Transaction, TransactionWithHash};
use blockifier::{execution::errors::ContractClassError, transaction::errors::TransactionExecutionError};
use mp_chain_config::StarknetVersion;
use mp_class::{
    compile::ClassCompilationError, CompressedLegacyContractClass, ConvertedClass, FlattenedSierraClass,
    LegacyClassInfo, LegacyConvertedClass, SierraClassInfo, SierraConvertedClass,
};
use starknet_api::transaction::TransactionHash;
use starknet_types_core::felt::Felt;

#[derive(thiserror::Error, Debug)]
pub enum BroadcastedToBlockifierError {
    #[error("Failed to compile contract class: {0}")]
    CompilationFailed(#[from] ClassCompilationError),
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
    #[error("Declare legacy contract classes are not supported")]
    LegacyContractClassesNotSupported,
    #[error("Compiled class hash mismatch: expected {expected}, actual {compilation}")]
    CompiledClassHashMismatch { expected: Felt, compilation: Felt },
}

pub fn broadcasted_to_blockifier(
    transaction: starknet_core::types::BroadcastedTransaction,
    chain_id: Felt,
    starknet_version: StarknetVersion,
) -> Result<
    (blockifier::transaction::transaction_execution::Transaction, Option<ConvertedClass>),
    BroadcastedToBlockifierError,
> {
    let (class_info, class_hash, extra_class_info) = match &transaction {
        starknet_core::types::BroadcastedTransaction::Declare(tx) => match tx {
            starknet_core::types::BroadcastedDeclareTransaction::V1(tx) => {
                let compressed_legacy_class: CompressedLegacyContractClass = (*tx.contract_class).clone().into();
                let class_hash = compressed_legacy_class.compute_class_hash().unwrap();
                log::debug!("Computed legacy class hash: {:?}", class_hash);
                let compressed_legacy_class: CompressedLegacyContractClass = (*tx.contract_class).clone().into();
                let class_blockifier = compressed_legacy_class
                    .to_blockifier_class()
                    .map_err(BroadcastedToBlockifierError::CompilationFailed)?;
                let class_info = LegacyClassInfo { contract_class: Arc::new(compressed_legacy_class) };

                (
                    Some(blockifier::execution::contract_class::ClassInfo::new(&class_blockifier, 0, 0)?),
                    Some(class_hash),
                    Some(ConvertedClass::Legacy(LegacyConvertedClass { class_hash, info: class_info })),
                )
            }
            starknet_core::types::BroadcastedDeclareTransaction::V2(tx) => {
                let class_hash = tx.contract_class.class_hash();
                let flatten_sierra_class: FlattenedSierraClass = (*tx.contract_class).clone().into();
                let (compiled_class_hash, compiled) = flatten_sierra_class.compile_to_casm()?;
                if tx.compiled_class_hash != compiled_class_hash {
                    return Err(BroadcastedToBlockifierError::CompiledClassHashMismatch {
                        expected: tx.compiled_class_hash,
                        compilation: compiled_class_hash,
                    });
                }
                let class_info =
                    SierraClassInfo { contract_class: Arc::new(flatten_sierra_class), compiled_class_hash };

                (
                    Some(blockifier::execution::contract_class::ClassInfo::new(
                        &compiled.to_blockifier_class()?,
                        tx.contract_class.sierra_program.len(),
                        tx.contract_class.abi.len(),
                    )?),
                    Some(class_hash),
                    Some(ConvertedClass::Sierra(SierraConvertedClass {
                        class_hash,
                        info: class_info,
                        compiled: Arc::new(compiled),
                    })),
                )
            }
            starknet_core::types::BroadcastedDeclareTransaction::V3(tx) => {
                let class_hash = tx.contract_class.class_hash();
                let flatten_sierra_class: FlattenedSierraClass = (*tx.contract_class).clone().into();
                let (compiled_class_hash, compiled) = flatten_sierra_class.compile_to_casm()?;
                if tx.compiled_class_hash != compiled_class_hash {
                    return Err(BroadcastedToBlockifierError::CompiledClassHashMismatch {
                        expected: tx.compiled_class_hash,
                        compilation: compiled_class_hash,
                    });
                }
                let class_info =
                    SierraClassInfo { contract_class: Arc::new(flatten_sierra_class), compiled_class_hash };

                (
                    Some(blockifier::execution::contract_class::ClassInfo::new(
                        &compiled.to_blockifier_class()?,
                        tx.contract_class.sierra_program.len(),
                        tx.contract_class.abi.len(),
                    )?),
                    Some(class_hash),
                    Some(ConvertedClass::Sierra(SierraConvertedClass {
                        class_hash,
                        info: class_info,
                        compiled: Arc::new(compiled),
                    })),
                )
            }
        },
        _ => (None, None, None),
    };

    let is_query = is_query(&transaction);
    let TransactionWithHash { transaction, hash } =
        TransactionWithHash::from_broadcasted(transaction, chain_id, starknet_version, class_hash);
    let deployed_address = match &transaction {
        Transaction::DeployAccount(tx) => Some(tx.calculate_contract_address()),
        _ => None,
    };
    let transaction: starknet_api::transaction::Transaction = transaction.try_into()?;

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
