use crate::{
    RejectedTransactionError, RejectedTransactionErrorKind, SubmitTransaction, SubmitTransactionError,
    SubmitValidatedTransaction,
};
use async_trait::async_trait;
use blockifier::{
    blockifier::stateful_validator::StatefulValidatorError,
    transaction::{
        account_transaction::AccountTransaction,
        errors::{TransactionExecutionError, TransactionPreValidationError},
        transaction_execution::Transaction as BTransaction,
    },
};
use mc_db::MadaraBackend;
use mc_exec::ExecutionContext;
use mp_class::ConvertedClass;
use mp_convert::{Felt, ToFelt};
use mp_rpc::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn,
    BroadcastedTxn, ClassAndTxnHash, ContractAndTxnHash,
};
use mp_transactions::{
    validated::{TxTimestamp, ValidatedMempoolTx},
    BroadcastedDeclareTransactionV0, BroadcastedTransactionExt, ToBlockifierError,
};
use starknet_api::executable_transaction::AccountTransaction as ApiExecutableTransaction;
use std::{borrow::Cow, sync::Arc};

fn rejected(kind: RejectedTransactionErrorKind, message: impl Into<Cow<'static, str>>) -> SubmitTransactionError {
    SubmitTransactionError::Rejected(RejectedTransactionError::new(kind, message))
}

impl From<TransactionPreValidationError> for SubmitTransactionError {
    fn from(err: TransactionPreValidationError) -> Self {
        use RejectedTransactionErrorKind::*;
        use TransactionPreValidationError as E;

        match err {
            E::InvalidNonce { .. } => rejected(InvalidTransactionNonce, format!("{err:#}")),
            // TODO: Some of those are internal server errors.
            err @ E::StateError(_) => rejected(ValidateFailure, format!("{err:#}")),
            err @ E::TransactionFeeError(_) => rejected(ValidateFailure, format!("{err:#}")),
        }
    }
}

impl From<StatefulValidatorError> for SubmitTransactionError {
    fn from(err: StatefulValidatorError) -> Self {
        use RejectedTransactionErrorKind::*;
        use StatefulValidatorError as E;

        match err {
            // TODO: Some of those are internal server errors.
            err @ E::StateError(_) => rejected(ValidateFailure, format!("{err:#}")),
            E::TransactionExecutionError(err) => err.into(),
            err @ E::TransactionExecutorError(_) => rejected(ValidateFailure, format!("{err:#}")),
            err @ E::TransactionPreValidationError(_) => err.into(),
        }
    }
}

impl From<TransactionExecutionError> for SubmitTransactionError {
    fn from(err: TransactionExecutionError) -> Self {
        use RejectedTransactionErrorKind::*;
        use TransactionExecutionError as E;

        match &err {
            E::ContractClassVersionMismatch { .. } => rejected(InvalidContractClassVersion, format!("{err:#}")),
            E::DeclareTransactionError { .. } => rejected(ClassAlreadyDeclared, format!("{err:#}")),
            E::ExecutionError { .. }
            | E::ValidateTransactionError { .. }
            | E::ContractConstructorExecutionFailed { .. } => rejected(ValidateFailure, format!("{err:#}")),
            E::FeeCheckError(_)
            | E::FromStr(_)
            | E::PanicInValidate { .. }
            | E::InvalidValidateReturnData { .. }
            | E::StarknetApiError(_)
            | E::TransactionFeeError(_)
            | E::TransactionPreValidationError(_)
            | E::TryFromIntError(_)
            | E::TransactionTooLarge { .. } => rejected(ValidateFailure, format!("{err:#}")),
            E::InvalidVersion { .. } => rejected(InvalidTransactionVersion, format!("{err:#}")),
            // Note: Unsure about InvalidSegmentStructure.
            E::ProgramError(_) | E::InvalidSegmentStructure(_, _) => rejected(InvalidProgram, format!("{err:#}")),
            // TODO: Some of those are internal server errors.
            E::StateError(_) => rejected(ValidateFailure, format!("{err:#}")),
        }
    }
}

impl From<ToBlockifierError> for SubmitTransactionError {
    fn from(value: ToBlockifierError) -> Self {
        use RejectedTransactionErrorKind::*;
        use ToBlockifierError as E;

        // These are not always precise or accurate.
        match value {
            E::CompilationFailed(class_compilation_error) => {
                rejected(CompilationFailed, format!("{class_compilation_error:#}"))
            }
            E::ProgramError(program_error) => rejected(InvalidProgram, format!("{program_error:#}")),
            E::ComputeLegacyClassHashFailed(error) => rejected(InvalidContractClass, format!("{error:#}")),
            E::ComputeSierraClassHashFailed(compute_class_hash_error) => {
                rejected(InvalidContractClass, format!("{compute_class_hash_error:#}"))
            }
            E::ConvertToTxApiError(transaction_api_error) => {
                rejected(ValidateFailure, format!("{transaction_api_error:#}"))
            }
            E::ConvertTxBlockifierError(transaction_execution_error) => {
                rejected(ValidateFailure, format!("{transaction_execution_error:#}"))
            }
            err @ E::CompiledClassHashMismatch { .. } => rejected(InvalidCompiledClassHash, format!("{err:#}")),
            E::Base64ToCairoError(error) => rejected(InvalidContractClass, format!("{error:#}")),
            E::MissingClass => rejected(InvalidContractClass, "Missing class"),
        }
    }
}

impl From<mc_exec::Error> for SubmitTransactionError {
    fn from(value: mc_exec::Error) -> Self {
        use mc_exec::Error as E;
        use RejectedTransactionErrorKind::*;
        use SubmitTransactionError::*;

        match &value {
            // these should not really happen here
            E::Reexecution(_) | E::FeeEstimation(_) | E::MessageFeeEstimation(_) | E::CallContract(_) => {
                rejected(ValidateFailure, format!("{value:#}"))
            }
            E::UnsupportedProtocolVersion(_) | E::Storage(_) | E::InvalidSequencerAddress(_) => {
                Internal(anyhow::anyhow!(value))
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct TransactionValidatorConfig {
    pub disable_validation: bool,
}
impl TransactionValidatorConfig {
    pub fn with_disable_validation(mut self, disable_validation: bool) -> Self {
        self.disable_validation = disable_validation;
        self
    }
}

pub struct TransactionValidator {
    inner: Arc<dyn SubmitValidatedTransaction>,
    backend: Arc<MadaraBackend>,
    config: TransactionValidatorConfig,
}

impl TransactionValidator {
    pub fn new(
        inner: Arc<dyn SubmitValidatedTransaction>,
        backend: Arc<MadaraBackend>,
        config: TransactionValidatorConfig,
    ) -> Self {
        Self { inner, backend, config }
    }

    #[tracing::instrument(skip(self, tx, converted_class), fields(module = "TxValidation"))]
    async fn accept_tx(
        &self,
        tx: BTransaction,
        converted_class: Option<ConvertedClass>,
        arrived_at: TxTimestamp,
    ) -> Result<(), SubmitTransactionError> {
        if is_only_query(&tx) {
            return Err(RejectedTransactionError::new(
                RejectedTransactionErrorKind::InvalidTransactionVersion,
                "Cannot submit query-only transactions",
            )
            .into());
        }
        // // If the contract has been deployed in the mempool but not in a pendng block, we need to skip some of the validations.
        // // This is blockifier's role, but it can't know of this edge case by itself so we have to tell it.
        // // NB: the lock is NOT taken the entire time the tx is being validated. As such, the deploy tx
        // //  may appear during that time - but it should not be a big problem.
        // let deploy_account_skip_validation =
        //     if let BTransaction::Account(AccountTransaction { tx: ApiExecutableTransaction::Invoke(tx), .. }) = &tx {
        //         let mempool = self.inner.read().expect("Poisoned lock");
        //         mempool.has_deployed_contract(&tx.tx.sender_address())
        //     } else {
        //         false
        //     };

        let deploy_account_skip_validation =
            matches!(tx, BTransaction::Account(AccountTransaction { tx: ApiExecutableTransaction::Invoke(_), .. }))
                && tx.nonce().to_felt() == Felt::TWO;

        let tx_hash = BTransaction::tx_hash(&tx).0;

        if !self.config.disable_validation {
            tracing::debug!("Mempool verify tx_hash={:#x}", tx_hash);

            // Perform validations
            if let BTransaction::Account(account_tx) = tx.clone() {
                let exec_context = ExecutionContext::new_on_pending(Arc::clone(&self.backend))?;
                let mut validator = exec_context.tx_validator();
                validator.perform_validations(account_tx, deploy_account_skip_validation)?
            }
        }

        // Forward the validated tx.
        let tx = ValidatedMempoolTx::from_blockifier(tx, arrived_at, converted_class);
        self.inner.submit_validated_transaction(tx).await?;

        Ok(())
    }
}

fn is_only_query(tx: &BTransaction) -> bool {
    match tx {
        BTransaction::Account(account_tx) => account_tx.execution_flags.only_query,
        BTransaction::L1Handler(_) => false,
    }
}

fn declared_class_hash(tx: &BTransaction) -> Option<Felt> {
    match tx {
        BTransaction::Account(AccountTransaction { tx: ApiExecutableTransaction::Declare(tx), .. }) => {
            Some(*tx.class_hash())
        }
        _ => None,
    }
}

fn deployed_contract_address(tx: &BTransaction) -> Option<Felt> {
    match tx {
        BTransaction::Account(AccountTransaction { tx: ApiExecutableTransaction::DeployAccount(tx), .. }) => {
            Some(**tx.contract_address)
        }
        _ => None,
    }
}

#[async_trait]
impl SubmitTransaction for TransactionValidator {
    async fn submit_declare_v0_transaction(
        &self,
        tx: BroadcastedDeclareTransactionV0,
    ) -> Result<ClassAndTxnHash, SubmitTransactionError> {
        let arrived_at = TxTimestamp::now();
        let (btx, class) = tx.into_blockifier(
            self.backend.chain_config().chain_id.to_felt(),
            self.backend.chain_config().latest_protocol_version,
        )?;

        let res = ClassAndTxnHash {
            transaction_hash: BTransaction::tx_hash(&btx).to_felt(),
            class_hash: declared_class_hash(&btx).expect("Created transaction should be Declare"),
        };
        self.accept_tx(btx, class, arrived_at).await?;
        Ok(res)
    }

    async fn submit_declare_transaction(
        &self,
        tx: BroadcastedDeclareTxn,
    ) -> Result<ClassAndTxnHash, SubmitTransactionError> {
        let arrived_at = TxTimestamp::now();
        let tx = BroadcastedTxn::Declare(tx);
        let (btx, class) = tx.into_blockifier(
            self.backend.chain_config().chain_id.to_felt(),
            self.backend.chain_config().latest_protocol_version,
            true,
            true,
        )?;

        let res = ClassAndTxnHash {
            transaction_hash: BTransaction::tx_hash(&btx).to_felt(),
            class_hash: declared_class_hash(&btx).expect("Created transaction should be Declare"),
        };
        self.accept_tx(btx, class, arrived_at).await?;
        Ok(res)
    }

    async fn submit_deploy_account_transaction(
        &self,
        tx: BroadcastedDeployAccountTxn,
    ) -> Result<ContractAndTxnHash, SubmitTransactionError> {
        let arrived_at = TxTimestamp::now();
        let tx = BroadcastedTxn::DeployAccount(tx);
        let (btx, class) = tx.into_blockifier(
            self.backend.chain_config().chain_id.to_felt(),
            self.backend.chain_config().latest_protocol_version,
            true,
            true,
        )?;

        let res = ContractAndTxnHash {
            transaction_hash: BTransaction::tx_hash(&btx).to_felt(),
            contract_address: deployed_contract_address(&btx).expect("Created transaction should be DeployAccount"),
        };
        self.accept_tx(btx, class, arrived_at).await?;
        Ok(res)
    }

    async fn submit_invoke_transaction(
        &self,
        tx: BroadcastedInvokeTxn,
    ) -> Result<AddInvokeTransactionResult, SubmitTransactionError> {
        let arrived_at = TxTimestamp::now();
        let tx = BroadcastedTxn::Invoke(tx);
        let (btx, class) = tx.into_blockifier(
            self.backend.chain_config().chain_id.to_felt(),
            self.backend.chain_config().latest_protocol_version,
            true,
            true,
        )?;

        let res = AddInvokeTransactionResult { transaction_hash: BTransaction::tx_hash(&btx).to_felt() };
        self.accept_tx(btx, class, arrived_at).await?;
        Ok(res)
    }
}
