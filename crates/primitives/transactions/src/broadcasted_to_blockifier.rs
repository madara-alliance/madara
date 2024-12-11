use crate::{
    from_broadcasted_transaction::is_query, into_starknet_api::TransactionApiError, BroadcastedDeclareTransactionV0,
    L1HandlerTransaction, Transaction, TransactionWithHash,
};
use blockifier::transaction::account_transaction::ExecutionFlags as AccountExecutionFlags;
use blockifier::{
    transaction::errors::TransactionExecutionError, transaction::transaction_execution::Transaction as BTransaction,
};
use mp_chain_config::StarknetVersion;
use mp_class::{
    class_hash, compile::ClassCompilationError, CompressedLegacyContractClass, ConvertedClass, FlattenedSierraClass,
    LegacyClassInfo, LegacyConvertedClass, SierraClassInfo, SierraConvertedClass,
};
use starknet_api::contract_class::ClassInfo as ApiClassInfo;
use starknet_api::contract_class::ContractClass as ApiContractClass;
use starknet_api::contract_class::SierraVersion;
use starknet_api::transaction::{fields::Fee, TransactionHash};
use starknet_types_core::felt::Felt;
use starknet_types_rpc::{BroadcastedDeclareTxn, BroadcastedTxn};
use std::sync::Arc;

pub trait BroadcastedTransactionExt {
    fn into_blockifier(
        self,
        chain_id: Felt,
        starknet_version: StarknetVersion,
        charge_fee: bool,
        validate: bool,
    ) -> Result<(BTransaction, Option<ConvertedClass>), BroadcastedToBlockifierError>;
}

impl BroadcastedTransactionExt for BroadcastedTxn<Felt> {
    fn into_blockifier(
        self,
        chain_id: Felt,
        starknet_version: StarknetVersion,
        charge_fee: bool,
        validate: bool,
    ) -> Result<(BTransaction, Option<ConvertedClass>), BroadcastedToBlockifierError> {
        let (class_info, converted_class, class_hash) = match &self {
            BroadcastedTxn::Declare(tx) => match tx {
                BroadcastedDeclareTxn::V1(tx) | BroadcastedDeclareTxn::QueryV1(tx) => {
                    handle_class_legacy(Arc::new((tx.contract_class).clone().try_into()?))?
                }
                BroadcastedDeclareTxn::V2(tx) | BroadcastedDeclareTxn::QueryV2(tx) => {
                    handle_class_sierra(Arc::new((tx.contract_class).clone().into()), tx.compiled_class_hash)?
                }
                BroadcastedDeclareTxn::V3(tx) | BroadcastedDeclareTxn::QueryV3(tx) => {
                    handle_class_sierra(Arc::new((tx.contract_class).clone().into()), tx.compiled_class_hash)?
                }
            },
            _ => (None, None, None),
        };

        let only_query = is_query(&self);
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
                AccountExecutionFlags { only_query, charge_fee, validate },
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
            BTransaction::from_api(
                transaction,
                TransactionHash(hash),
                None,
                Some(Fee(paid_fees_on_l1)),
                None,
                AccountExecutionFlags::default(),
            )?,
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
        let transaction = Transaction::Declare(crate::DeclareTransaction::from_broadcasted_declare_v0(
            self,
            class_hash.expect("Class hash must be provided for DeclareTransaction"),
        ));
        let hash = transaction.compute_hash(chain_id, starknet_version, is_query);
        let transaction: starknet_api::transaction::Transaction = transaction.try_into()?;

        Ok((
            BTransaction::from_api(
                transaction,
                TransactionHash(hash),
                class_info,
                None,
                None,
                execution_flags(is_query),
            )?,
            converted_class,
        ))
    }
}

fn execution_flags(only_query: bool) -> AccountExecutionFlags {
    AccountExecutionFlags { only_query, ..Default::default() }
}

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
    #[error("Compiled class hash mismatch: expected {expected}, actual {compilation}")]
    CompiledClassHashMismatch { expected: Felt, compilation: Felt },
    #[error("Failed to convert base64 program to cairo program: {0}")]
    Base64ToCairoError(#[from] base64::DecodeError),
}

#[allow(clippy::type_complexity)]
fn handle_class_legacy(
    contract_class: Arc<CompressedLegacyContractClass>,
) -> Result<(Option<ApiClassInfo>, Option<ConvertedClass>, Option<Felt>), BroadcastedToBlockifierError> {
    let class_hash = contract_class.compute_class_hash()?;
    tracing::debug!("Computed legacy class hash: {:?}", class_hash);
    let class_api = ApiContractClass::V0(
        contract_class.to_starknet_api_no_abi().map_err(BroadcastedToBlockifierError::CompilationFailed)?,
    );
    Ok((
        Some(ApiClassInfo {
            contract_class: class_api,
            sierra_program_length: 0,
            abi_length: 0,
            sierra_version: SierraVersion::DEPRECATED,
        }),
        Some(ConvertedClass::Legacy(LegacyConvertedClass { class_hash, info: LegacyClassInfo { contract_class } })),
        Some(class_hash),
    ))
}

#[allow(clippy::type_complexity)]
fn handle_class_sierra(
    contract_class: Arc<FlattenedSierraClass>,
    expected_compiled_class_hash: Felt,
) -> Result<(Option<ApiClassInfo>, Option<ConvertedClass>, Option<Felt>), BroadcastedToBlockifierError> {
    let sierra_program_length = contract_class.program_length();
    let abi_length = contract_class.abi_length();
    let sierra_version = contract_class.sierra_version()?;
    let class_hash = contract_class.compute_class_hash()?;
    let (compiled_class_hash, compiled) = contract_class.compile_to_casm()?;
    let json_compiled = (&compiled).try_into()?;
    let class_api = ApiContractClass::V1(compiled);
    if expected_compiled_class_hash != compiled_class_hash {
        return Err(BroadcastedToBlockifierError::CompiledClassHashMismatch {
            expected: expected_compiled_class_hash,
            compilation: compiled_class_hash,
        });
    }
    Ok((
        Some(ApiClassInfo { contract_class: class_api, sierra_program_length, abi_length, sierra_version }),
        Some(ConvertedClass::Sierra(SierraConvertedClass {
            class_hash,
            info: SierraClassInfo { contract_class, compiled_class_hash },
            compiled: Arc::new(json_compiled),
        })),
        Some(class_hash),
    ))
}
