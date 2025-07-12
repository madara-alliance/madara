use crate::{into_starknet_api::TransactionApiError, L1HandlerTransaction, Transaction, TransactionWithHash};
use blockifier::transaction::account_transaction::ExecutionFlags;
use blockifier::{
    transaction::errors::TransactionExecutionError, transaction::transaction_execution::Transaction as BTransaction,
};
use mp_chain_config::StarknetVersion;
use mp_class::{
    class_hash, compile::ClassCompilationError, CompressedLegacyContractClass, ConvertedClass, FlattenedSierraClass,
    LegacyClassInfo, LegacyConvertedClass, SierraClassInfo, SierraConvertedClass,
};
use mp_rpc::admin::BroadcastedDeclareTxnV0;
use mp_rpc::{BroadcastedDeclareTxn, BroadcastedTxn};
use starknet_api::contract_class::ClassInfo as ApiClassInfo;
use starknet_api::core::ContractAddress;
use starknet_api::executable_transaction::{
    AccountTransaction as ApiAccountTransaction, DeclareTransaction as ApiDeclareTransaction,
    DeployAccountTransaction as ApiDeployAccountTransaction, InvokeTransaction as ApiInvokeTransaction,
};
use starknet_api::transaction::{fields::Fee, TransactionHash};
use starknet_types_core::felt::Felt;
use std::sync::Arc;

impl TransactionWithHash {
    /// Very important note: When the transaction is an L1HandlerTransaction, the paid_fee_on_l1 field will be set to
    /// a very high value, as it is not stored in the transaction. This field does not affect the execution except
    /// that it may lead to a rejection on L2. (L1HandlerTransactions are not revertible) This means that this
    /// implementation is fine as long as the transaction has been checked beforehand.
    /// TODO: check that this is always true.
    ///
    /// Callers of this function must make sure that the transaction has originally been executed with the correct,
    /// paid_fee_on_l1 field.
    ///
    /// In madara, there are currently two places where this function is executed:
    /// - in RPCs, to replay (trace) transactions.
    /// - in block production, to replay the pending block when restarting sequencing.
    ///
    /// In the first case, we can't always get the paid_fees_on_l1 field, but since the sequencer supposedly has
    /// executed this transaction before, it's fine to suppose that it's valid.
    /// In the second case, this transaction has already been validated by the mempool and block production.
    pub fn into_blockifier(self, class: Option<&ConvertedClass>) -> Result<BTransaction, ToBlockifierError> {
        let class_info = match &self.transaction {
            Transaction::Declare(_txn) => {
                let class = class.ok_or(ToBlockifierError::MissingClass)?;
                Some(class.try_into()?)
            }
            _ => None,
        };

        // see doc comment
        let paid_fee_on_l1 =
            self.transaction.as_l1_handler().map(|_| starknet_api::transaction::fields::Fee(1_000_000_000_000));

        let deployed_address = match &self.transaction {
            // todo: this shouldnt be computed here...
            Transaction::DeployAccount(tx) => Some(tx.calculate_contract_address()),
            _ => None,
        };
        let transaction: starknet_api::transaction::Transaction = self.transaction.try_into()?;

        Ok(BTransaction::from_api(
            transaction,
            TransactionHash(self.hash),
            class_info,
            paid_fee_on_l1,
            deployed_address.map(|address| address.try_into().expect("Address conversion should never fail")),
            ExecutionFlags::default(),
        )?)
    }
}

pub trait BroadcastedTransactionExt {
    fn into_starknet_api(
        self,
        chain_id: Felt,
        starknet_version: StarknetVersion,
    ) -> Result<(ApiAccountTransaction, Option<ConvertedClass>), ToBlockifierError>;
}

impl BroadcastedTransactionExt for BroadcastedTxn {
    fn into_starknet_api(
        self,
        chain_id: Felt,
        starknet_version: StarknetVersion,
    ) -> Result<(ApiAccountTransaction, Option<ConvertedClass>), ToBlockifierError> {
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

        let TransactionWithHash { transaction, hash } =
            TransactionWithHash::from_broadcasted(self, chain_id, starknet_version, class_hash);
        let deployed_address = match &transaction {
            Transaction::DeployAccount(tx) => Some(tx.calculate_contract_address()),
            _ => None,
        };
        // println!(">>>>> 1. txn inside the into_starknet_api is: {:?}", transaction);
        let transaction = match transaction {
            Transaction::Declare(tx) => ApiAccountTransaction::Declare(ApiDeclareTransaction {
                tx: tx.try_into()?,
                tx_hash: TransactionHash(hash),
                class_info: class_info.expect("BroadcastedDeclareTxn generate a ClassInfo"),
            }),
            Transaction::DeployAccount(tx) => ApiAccountTransaction::DeployAccount(ApiDeployAccountTransaction {
                tx: tx.try_into()?,
                tx_hash: TransactionHash(hash),
                contract_address: ContractAddress(
                    deployed_address
                        .expect("BroadcastedDeployAccount generate a DeployedAddress")
                        .try_into()
                        .expect("Calculated deployed_address is in bound"),
                ),
            }),
            Transaction::Invoke(tx) => ApiAccountTransaction::Invoke(ApiInvokeTransaction {
                tx: tx.try_into()?,
                tx_hash: TransactionHash(hash),
            }),
            Transaction::L1Handler(_) | Transaction::Deploy(_) => {
                unreachable!("BroadcastedTxn can't be L1Handler or Deploy")
            }
        };
        // println!(">>>>> 2. txn inside the into_starknet_api is: {:?}", transaction);

        Ok((transaction, converted_class))
    }
}

impl L1HandlerTransaction {
    pub fn into_blockifier(
        self,
        chain_id: Felt,
        _starknet_version: StarknetVersion,
        paid_fees_on_l1: u128,
    ) -> Result<(BTransaction, Option<ConvertedClass>), ToBlockifierError> {
        let transaction = Transaction::L1Handler(self.clone());
        // TODO: check self.version
        let hash = self.compute_hash(chain_id, /* offset_version */ false, /* legacy */ false);
        let transaction: starknet_api::transaction::Transaction = transaction.try_into()?;

        Ok((
            BTransaction::from_api(
                transaction,
                TransactionHash(hash),
                None,
                Some(Fee(paid_fees_on_l1)),
                None,
                ExecutionFlags::default(),
            )?,
            None,
        ))
    }
}

impl BroadcastedTransactionExt for BroadcastedDeclareTxnV0 {
    fn into_starknet_api(
        self,
        chain_id: Felt,
        _starknet_version: StarknetVersion,
    ) -> Result<(ApiAccountTransaction, Option<ConvertedClass>), ToBlockifierError> {
        let (class_info, converted_class, class_hash) =
            handle_class_legacy(Arc::new((self.contract_class).clone().try_into()?))?;

        let is_query = self.is_query;
        let transaction = crate::DeclareTransaction::from_broadcasted_declare_v0(
            self,
            class_hash.expect("Class hash must be provided for DeclareTransaction"),
        );
        let hash = transaction.compute_hash(chain_id, is_query);
        // let transaction: starknet_api::transaction::Transaction = transaction.try_into()?;
        let transaction = ApiAccountTransaction::Declare(ApiDeclareTransaction {
            tx: transaction.try_into()?,
            tx_hash: TransactionHash(hash),
            class_info: class_info.expect("BroadcastedDeclareTxnV0 generate a ClassInfo"),
        });

        Ok((transaction, converted_class))
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ToBlockifierError {
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
    Base64ToCairoError(#[from] std::io::Error),
    #[error("Missing class")]
    MissingClass,
    #[error("Failed to convert class to api: {0}")]
    ConvertClassToApiError(#[from] serde_json::Error),
}

#[allow(clippy::type_complexity)]
fn handle_class_legacy(
    contract_class: Arc<CompressedLegacyContractClass>,
) -> Result<(Option<ApiClassInfo>, Option<ConvertedClass>, Option<Felt>), ToBlockifierError> {
    let class_hash = contract_class.compute_class_hash()?;
    tracing::debug!("Computed legacy class hash: {:?}", class_hash);
    let converted_class =
        ConvertedClass::Legacy(LegacyConvertedClass { class_hash, info: LegacyClassInfo { contract_class } });
    let api_class_info = (&converted_class).try_into()?;
    Ok((Some(api_class_info), Some(converted_class), Some(class_hash)))
}

#[allow(clippy::type_complexity)]
fn handle_class_sierra(
    contract_class: Arc<FlattenedSierraClass>,
    expected_compiled_class_hash: Felt,
) -> Result<(Option<ApiClassInfo>, Option<ConvertedClass>, Option<Felt>), ToBlockifierError> {
    let class_hash = contract_class.compute_class_hash()?;
    let (compiled_class_hash, compiled) = contract_class.compile_to_casm()?;
    if expected_compiled_class_hash != compiled_class_hash {
        return Err(ToBlockifierError::CompiledClassHashMismatch {
            expected: expected_compiled_class_hash,
            compilation: compiled_class_hash,
        });
    }
    let converted_class = ConvertedClass::Sierra(SierraConvertedClass {
        class_hash,
        info: SierraClassInfo { contract_class, compiled_class_hash },
        compiled: Arc::new((&compiled).try_into()?),
    });
    let api_class_info = (&converted_class).try_into()?;
    Ok((Some(api_class_info), Some(converted_class), Some(class_hash)))
}
