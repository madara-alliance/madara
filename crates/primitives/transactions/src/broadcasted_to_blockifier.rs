use crate::{into_starknet_api::TransactionApiError, L1HandlerTransaction, Transaction, TransactionWithHash};
use crate::{BroadcastedDeclareTransactionV0, DeclareTransaction};
use blockifier::{
    execution::contract_class::ClassInfo as BClassInfo, transaction::transaction_execution::Transaction as BTransaction,
};
use starknet_core::types::{
    BroadcastedDeclareTransaction, BroadcastedDeployAccountTransaction, BroadcastedInvokeTransaction,
    BroadcastedTransaction,
};

use blockifier::{execution::errors::ContractClassError, transaction::errors::TransactionExecutionError};
use mp_chain_config::StarknetVersion;
use mp_class::{
    class_hash::ComputeClassHashError, compile::ClassCompilationError, CompressedLegacyContractClass, ConvertedClass,
    FlattenedSierraClass, LegacyClassInfo, LegacyConvertedClass, SierraClassInfo, SierraConvertedClass,
};
use starknet_api::transaction::{Fee, TransactionHash};
use starknet_types_core::felt::Felt;
use std::sync::Arc;

pub trait BroadcastedTransactionExt {
    fn into_blockifier(
        self,
        chain_id: Felt,
        starknet_version: StarknetVersion,
    ) -> Result<(BTransaction, Option<ConvertedClass>), BroadcastedToBlockifierError>;
}

pub fn is_query(tx: &BroadcastedTransaction) -> bool {
    match tx {
        BroadcastedTransaction::Invoke(tx) => match tx {
            BroadcastedInvokeTransaction::V1(tx) => tx.is_query,
            BroadcastedInvokeTransaction::V3(tx) => tx.is_query,
        },
        BroadcastedTransaction::Declare(tx) => match tx {
            BroadcastedDeclareTransaction::V1(tx) => tx.is_query,
            BroadcastedDeclareTransaction::V2(tx) => tx.is_query,
            BroadcastedDeclareTransaction::V3(tx) => tx.is_query,
        },
        BroadcastedTransaction::DeployAccount(tx) => match tx {
            BroadcastedDeployAccountTransaction::V1(tx) => tx.is_query,
            BroadcastedDeployAccountTransaction::V3(tx) => tx.is_query,
        },
    }
}

impl BroadcastedTransactionExt for BroadcastedTransaction {
    fn into_blockifier(
        self,
        chain_id: Felt,
        starknet_version: StarknetVersion,
    ) -> Result<(BTransaction, Option<ConvertedClass>), BroadcastedToBlockifierError> {
        let (class_info, converted_class, class_hash) = match &self {
            BroadcastedTransaction::Declare(tx) => match tx {
                BroadcastedDeclareTransaction::V1(tx) => {
                    handle_class_legacy(Arc::new((*tx.contract_class).clone().into()))?
                }
                BroadcastedDeclareTransaction::V2(tx) => {
                    handle_class_sierra(Arc::new((*tx.contract_class).clone().into()), tx.compiled_class_hash)?
                }
                BroadcastedDeclareTransaction::V3(tx) => {
                    handle_class_sierra(Arc::new((*tx.contract_class).clone().into()), tx.compiled_class_hash)?
                }
            },
            _ => (None, None, None),
        };

        let is_query = is_query(&self);
        let TransactionWithHash { transaction, hash } =
            TransactionWithHash::from_broadcasted(self, chain_id, starknet_version, class_hash);
        let deployed_address = match &transaction {
            Transaction::DeployAccount(tx) => Some(tx.calculate_contract_address()),
            _ => None,
        };
        let transaction: starknet_api::transaction::Transaction = transaction.try_into()?;

        Ok((
            BTransaction::from_api(
                transaction,
                TransactionHash(hash),
                class_info,
                None,
                deployed_address.map(|address| address.try_into().expect("Address conversion should never fail")),
                is_query,
            )?,
            converted_class,
        ))
    }
}

impl L1HandlerTransaction {
    pub fn into_blockifier(
        self,
        chain_id: Felt,
        _starknet_version: StarknetVersion,
        paid_fees_on_l1: u128,
    ) -> Result<(BTransaction, Option<ConvertedClass>), BroadcastedToBlockifierError> {
        let transaction = Transaction::L1Handler(self.clone());
        // TODO: check self.version
        let hash = self.compute_hash(chain_id, false, false);
        let transaction: starknet_api::transaction::Transaction = transaction.try_into()?;

        Ok((
            BTransaction::from_api(transaction, TransactionHash(hash), None, Some(Fee(paid_fees_on_l1)), None, false)?,
            None,
        ))
    }
}

impl BroadcastedDeclareTransactionV0 {
    pub fn into_blockifier(
        self,
        chain_id: Felt,
        starknet_version: StarknetVersion,
    ) -> Result<(BTransaction, Option<ConvertedClass>), BroadcastedToBlockifierError> {
        let (class_info, converted_class, class_hash) = handle_class_legacy(Arc::clone(&self.contract_class))?;

        let is_query = self.is_query;
        let transaction = Transaction::Declare(DeclareTransaction::from_broadcasted_declare_v0(
            self,
            class_hash.expect("Class hash must be provided for DeclareTransaction"),
        ));
        let hash = transaction.compute_hash(chain_id, starknet_version, is_query);
        let transaction: starknet_api::transaction::Transaction = transaction.try_into()?;

        Ok((
            BTransaction::from_api(transaction, TransactionHash(hash), class_info, None, None, is_query)?,
            converted_class,
        ))
    }
}

#[derive(thiserror::Error, Debug)]
pub enum BroadcastedToBlockifierError {
    #[error("Failed to compile contract class: {0}")]
    CompilationFailed(#[from] ClassCompilationError),
    #[error("Failed to convert program: {0}")]
    ProgramError(#[from] cairo_vm::types::errors::program_errors::ProgramError),
    #[error("Failed to compute legacy class hash: {0}")]
    ComputeLegacyClassHashFailed(#[from] ComputeClassHashError),
    #[error("Failed to convert transaction to starkneti-api: {0}")]
    ConvertToTxApiError(#[from] TransactionApiError),
    #[error("Failed to convert transaction to blockifier: {0}")]
    ConvertTxBlockifierError(#[from] TransactionExecutionError),
    #[error("Failed to convert contract class: {0}")]
    ConvertContractClassError(#[from] ContractClassError),
    #[error("Compiled class hash mismatch: expected {expected}, actual {compilation}")]
    CompiledClassHashMismatch { expected: Felt, compilation: Felt },
}

#[allow(clippy::type_complexity)]
fn handle_class_legacy(
    contract_class: Arc<CompressedLegacyContractClass>,
) -> Result<(Option<BClassInfo>, Option<ConvertedClass>, Option<Felt>), BroadcastedToBlockifierError> {
    let class_hash = contract_class.compute_class_hash()?;
    tracing::debug!("Computed legacy class hash: {:?}", class_hash);
    let class_blockifier =
        contract_class.to_blockifier_class().map_err(BroadcastedToBlockifierError::CompilationFailed)?;
    Ok((
        Some(BClassInfo::new(&class_blockifier, 0, 0)?),
        Some(ConvertedClass::Legacy(LegacyConvertedClass { class_hash, info: LegacyClassInfo { contract_class } })),
        Some(class_hash),
    ))
}

#[allow(clippy::type_complexity)]
fn handle_class_sierra(
    contract_class: Arc<FlattenedSierraClass>,
    expected_compiled_class_hash: Felt,
) -> Result<(Option<BClassInfo>, Option<ConvertedClass>, Option<Felt>), BroadcastedToBlockifierError> {
    let class_hash = contract_class.compute_class_hash()?;
    let (compiled_class_hash, compiled) = contract_class.compile_to_casm()?;
    if expected_compiled_class_hash != compiled_class_hash {
        return Err(BroadcastedToBlockifierError::CompiledClassHashMismatch {
            expected: expected_compiled_class_hash,
            compilation: compiled_class_hash,
        });
    }
    Ok((
        Some(BClassInfo::new(
            &compiled.to_blockifier_class()?,
            contract_class.sierra_program.len(),
            contract_class.abi.len(),
        )?),
        Some(ConvertedClass::Sierra(SierraConvertedClass {
            class_hash,
            info: SierraClassInfo { contract_class, compiled_class_hash },
            compiled: Arc::new(compiled),
        })),
        Some(class_hash),
    ))
}
