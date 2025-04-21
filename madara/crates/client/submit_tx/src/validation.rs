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
use mc_exec::{execution::TxInfo, ExecutionContext};
use mp_class::ConvertedClass;
use mp_convert::{Felt, ToFelt};
use mp_rpc::{
    admin::BroadcastedDeclareTxnV0, AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn,
    BroadcastedInvokeTxn, BroadcastedTxn, ClassAndTxnHash, ContractAndTxnHash,
};
use mp_transactions::{
    validated::{TxTimestamp, ValidatedMempoolTx},
    BroadcastedTransactionExt, ToBlockifierError,
};
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
            E::TransactionPreValidationError(err) => err.into(),
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
            | E::InvalidValidateReturnData { .. }
            | E::StarknetApiError(_)
            | E::TransactionFeeError(_)
            | E::TransactionPreValidationError(_)
            | E::TryFromIntError(_)
            | E::TransactionTooLarge { .. } => rejected(ValidateFailure, format!("{err:#}")),
            E::InvalidVersion { .. } => rejected(InvalidTransactionVersion, format!("{err:#}")),
            // Note: Unsure about InvalidSegmentStructure.
            E::InvalidSegmentStructure(_, _) => rejected(InvalidProgram, format!("{err:#}")),
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
            E::ConvertContractClassError(error) => rejected(InvalidContractClass, format!("{error:#}")),
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
        if tx.is_only_query() {
            return Err(RejectedTransactionError::new(
                RejectedTransactionErrorKind::InvalidTransactionVersion,
                "Cannot submit query-only transactions",
            )
            .into());
        }

        // We have to skip part of the validation in a the very specific case where you send an invoke tx directly after a deploy account:
        // the account is not deployed yet but the tx should be accepted.
        let deploy_account_skip_validation =
            matches!(tx, BTransaction::AccountTransaction(AccountTransaction::Invoke(_)))
                && tx.nonce().to_felt() == Felt::ONE;

        let tx_hash = tx.tx_hash().to_felt();

        if !self.config.disable_validation {
            tracing::debug!("Mempool verify tx_hash={:#x}", tx_hash);

            // Perform validations
            if let BTransaction::AccountTransaction(account_tx) = tx.clone_blockifier_transaction() {
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

#[async_trait]
impl SubmitTransaction for TransactionValidator {
    async fn submit_declare_v0_transaction(
        &self,
        tx: BroadcastedDeclareTxnV0,
    ) -> Result<ClassAndTxnHash, SubmitTransactionError> {
        let arrived_at = TxTimestamp::now();
        let (btx, class) = tx.into_blockifier(
            self.backend.chain_config().chain_id.to_felt(),
            self.backend.chain_config().latest_protocol_version,
        )?;

        let res = ClassAndTxnHash {
            transaction_hash: btx.tx_hash().to_felt(),
            class_hash: btx.declared_class_hash().expect("Created transaction should be Declare").to_felt(),
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
        )?;

        let res = ClassAndTxnHash {
            transaction_hash: btx.tx_hash().to_felt(),
            class_hash: btx.declared_class_hash().expect("Created transaction should be Declare").to_felt(),
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
        )?;

        let res = ContractAndTxnHash {
            transaction_hash: btx.tx_hash().to_felt(),
            contract_address: btx
                .deployed_contract_address()
                .expect("Created transaction should be DeployAccount")
                .to_felt(),
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
        )?;

        let res = AddInvokeTransactionResult { transaction_hash: btx.tx_hash().to_felt() };
        self.accept_tx(btx, class, arrived_at).await?;
        Ok(res)
    }
}
