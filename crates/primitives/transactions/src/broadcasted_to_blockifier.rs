use std::sync::Arc;

use crate::{
    from_broadcasted_transaction::is_query, into_starknet_api::TransactionApiError, BroadcastedDeclareTransactionV0,
    Transaction, TransactionWithHash,
};
use blockifier::{execution::errors::ContractClassError, transaction::errors::TransactionExecutionError};
use mp_chain_config::StarknetVersion;
use mp_class::{
    class_hash, compile::ClassCompilationError, CompressedLegacyContractClass, ConvertedClass, FlattenedSierraClass,
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
    #[error("Failed to compute sierra class hash: {0}")]
    ComputeSierraClassHashFailed(#[from] class_hash::ComputeClassHashError),
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
    #[error("Failed to convert base64 program to cairo program: {0}")]
    Base64ToCairoError(#[from] base64::DecodeError),
}

pub fn broadcasted_declare_v0_to_blockifier(
    transaction: BroadcastedDeclareTransactionV0,
    chain_id: Felt,
    starknet_version: StarknetVersion,
) -> Result<
    (blockifier::transaction::transaction_execution::Transaction, Option<ConvertedClass>),
    BroadcastedToBlockifierError,
> {
    let (class_info, class_hash, extra_class_info) = {
        let compressed_legacy_class: CompressedLegacyContractClass = (*transaction.contract_class).clone();
        let class_hash = compressed_legacy_class.compute_class_hash().unwrap();
        let compressed_legacy_class: CompressedLegacyContractClass = (*transaction.contract_class).clone();
        let class_blockifier =
            compressed_legacy_class.to_blockifier_class().map_err(BroadcastedToBlockifierError::CompilationFailed)?;

        let class_info = LegacyClassInfo { contract_class: Arc::new(compressed_legacy_class) };

        (
            Some(blockifier::execution::contract_class::ClassInfo::new(&class_blockifier, 0, 0)?),
            Some(class_hash),
            Some(ConvertedClass::Legacy(LegacyConvertedClass { class_hash, info: class_info })),
        )
    };

    let is_query = transaction.is_query;
    let TransactionWithHash { transaction, hash } =
        TransactionWithHash::from_broadcasted_declare_v0(transaction, chain_id, starknet_version, class_hash);

    let transaction: starknet_api::transaction::Transaction = transaction.try_into()?;

    Ok((
        blockifier::transaction::transaction_execution::Transaction::from_api(
            transaction,
            TransactionHash(hash),
            class_info,
            None,
            None,
            is_query,
        )?,
        extra_class_info,
    ))
}

pub fn broadcasted_to_blockifier(
    transaction: starknet_types_rpc::BroadcastedTxn<Felt>,
    chain_id: Felt,
    starknet_version: StarknetVersion,
) -> Result<
    (blockifier::transaction::transaction_execution::Transaction, Option<ConvertedClass>),
    BroadcastedToBlockifierError,
> {
    let (class_info, class_hash, extra_class_info) = match &transaction {
        starknet_types_rpc::BroadcastedTxn::Declare(tx) => match tx {
            starknet_types_rpc::BroadcastedDeclareTxn::V1(starknet_types_rpc::BroadcastedDeclareTxnV1 {
                contract_class,
                ..
            })
            | starknet_types_rpc::BroadcastedDeclareTxn::QueryV1(starknet_types_rpc::BroadcastedDeclareTxnV1 {
                contract_class,
                ..
            }) => {
                let compressed_legacy_class: CompressedLegacyContractClass = contract_class.clone().try_into()?;
                let class_hash = compressed_legacy_class.compute_class_hash().unwrap();
                tracing::debug!("Computed legacy class hash: {:?}", class_hash);
                let compressed_legacy_class: CompressedLegacyContractClass = contract_class.clone().try_into()?;
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
            starknet_types_rpc::BroadcastedDeclareTxn::V2(starknet_types_rpc::BroadcastedDeclareTxnV2 {
                compiled_class_hash,
                contract_class,
                ..
            })
            | starknet_types_rpc::BroadcastedDeclareTxn::QueryV2(starknet_types_rpc::BroadcastedDeclareTxnV2 {
                compiled_class_hash,
                contract_class,
                ..
            })
            | starknet_types_rpc::BroadcastedDeclareTxn::V3(starknet_types_rpc::BroadcastedDeclareTxnV3 {
                compiled_class_hash,
                contract_class,
                ..
            })
            | starknet_types_rpc::BroadcastedDeclareTxn::QueryV3(starknet_types_rpc::BroadcastedDeclareTxnV3 {
                compiled_class_hash,
                contract_class,
                ..
            }) => {
                let flatten_sierra_class: FlattenedSierraClass = contract_class.clone().into();
                let class_hash = flatten_sierra_class
                    .compute_class_hash()
                    .map_err(BroadcastedToBlockifierError::ComputeSierraClassHashFailed)?;
                let (compiled_class_hash_computed, compiled) = flatten_sierra_class.compile_to_casm()?;
                if compiled_class_hash != &compiled_class_hash_computed {
                    return Err(BroadcastedToBlockifierError::CompiledClassHashMismatch {
                        expected: *compiled_class_hash,
                        compilation: compiled_class_hash_computed,
                    });
                }
                let class_info = SierraClassInfo {
                    contract_class: Arc::new(flatten_sierra_class),
                    compiled_class_hash: compiled_class_hash_computed,
                };

                (
                    Some(blockifier::execution::contract_class::ClassInfo::new(
                        &compiled.to_blockifier_class()?,
                        contract_class.sierra_program.len(),
                        contract_class.abi.as_ref().map(|abi| abi.len()).unwrap_or(0),
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
            deployed_address.map(|address| address.try_into().expect("Address conversion should never fail")),
            is_query,
        )?,
        extra_class_info,
    ))
}
