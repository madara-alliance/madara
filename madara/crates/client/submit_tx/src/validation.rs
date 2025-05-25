use crate::{
    RejectedTransactionError, RejectedTransactionErrorKind, SubmitTransaction, SubmitTransactionError,
    SubmitValidatedTransaction,
};
use async_trait::async_trait;
use blockifier::{
    blockifier::{stateful_validator::StatefulValidatorError, transaction_executor::TransactionExecutorError},
    state::errors::StateError,
    transaction::{
        errors::{TransactionExecutionError, TransactionPreValidationError},
        transaction_execution::Transaction as BTransaction,
        transaction_types::TransactionType,
    },
};
use mc_db::MadaraBackend;
use mc_exec::{execution::TxInfo, MadaraBackendExecutionExt};
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

impl From<StateError> for SubmitTransactionError {
    fn from(err: StateError) -> Self {
        use RejectedTransactionErrorKind::*;
        use StateError as E;
        use SubmitTransactionError::*;

        match err {
            // Any error raised in a madara BlockifierStateAdaptor accessor.
            E::StateReadError(err) => Internal(anyhow::anyhow!(err)),
            err => rejected(ValidateFailure, format!("{err:#}")),
        }
    }
}

impl From<TransactionPreValidationError> for SubmitTransactionError {
    fn from(err: TransactionPreValidationError) -> Self {
        use RejectedTransactionErrorKind::*;
        use TransactionPreValidationError as E;

        match err {
            E::InvalidNonce { .. } => rejected(InvalidTransactionNonce, format!("{err:#}")),
            E::StateError(err) => err.into(),
            err @ E::TransactionFeeError(_) => rejected(ValidateFailure, format!("{err:#}")),
        }
    }
}

impl From<StatefulValidatorError> for SubmitTransactionError {
    fn from(err: StatefulValidatorError) -> Self {
        use StatefulValidatorError as E;

        match err {
            E::StateError(err) => err.into(),
            E::TransactionExecutionError(err) => err.into(),
            E::TransactionExecutorError(err) => err.into(),
            E::TransactionPreValidationError(err) => err.into(),
        }
    }
}

impl From<TransactionExecutorError> for SubmitTransactionError {
    fn from(err: TransactionExecutorError) -> Self {
        use RejectedTransactionErrorKind::*;
        use TransactionExecutorError as E;

        match err {
            E::BlockFull => rejected(ValidateFailure, format!("{err:#}")),
            E::StateError(err) => err.into(),
            E::TransactionExecutionError(err) => err.into(),
            E::CompressionError(err) => rejected(ValidateFailure, format!("{err:#}")),
        }
    }
}

impl From<TransactionExecutionError> for SubmitTransactionError {
    fn from(err: TransactionExecutionError) -> Self {
        use RejectedTransactionErrorKind::*;
        use TransactionExecutionError as E;

        match err {
            err @ E::ContractClassVersionMismatch { .. } => rejected(InvalidContractClassVersion, format!("{err:#}")),
            err @ E::DeclareTransactionError { .. } => rejected(ClassAlreadyDeclared, format!("{err:#}")),
            err @ (E::ExecutionError { .. }
            | E::ValidateTransactionError { .. }
            | E::ContractConstructorExecutionFailed { .. }
            | E::PanicInValidate { .. }) => rejected(ValidateFailure, format!("{err:#}")),
            err @ (E::FeeCheckError(_)
            | E::FromStr(_)
            | E::InvalidValidateReturnData { .. }
            | E::StarknetApiError(_)
            | E::TransactionFeeError(_)
            | E::TransactionPreValidationError(_)
            | E::TryFromIntError(_)
            | E::TransactionTooLarge) => rejected(ValidateFailure, format!("{err:#}")),
            err @ E::InvalidVersion { .. } => rejected(InvalidTransactionVersion, format!("{err:#}")),
            err @ (E::InvalidSegmentStructure(_, _) | E::ProgramError { .. }) => {
                rejected(InvalidProgram, format!("{err:#}"))
            }
            E::StateError(err) => err.into(),
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
            err @ E::Base64ToCairoError(_) => rejected(InvalidContractClass, format!("{err:#}")),
            E::ConvertClassToApiError(error) => rejected(InvalidContractClass, format!("{error:#}")),
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

        // We have to skip part of the validation in the very specific case where you send an invoke tx directly after a deploy account:
        // the account is not deployed yet but the tx should be accepted.
        let deploy_account_skip_validation =
            tx.tx_type() == TransactionType::InvokeFunction && tx.nonce().to_felt() == Felt::ONE;

        let tx_hash = tx.tx_hash().to_felt();

        if !self.config.disable_validation {
            tracing::debug!("Mempool verify tx_hash={:#x}", tx_hash);

            // Perform validations
            if let BTransaction::Account(account_tx) = tx.clone() {
                let mut validator = self.backend.new_transaction_validator()?;
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
            true,
            true, // TODO: did we want disabled charge fee for declare v0?
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
        let validate = !tx.is_query(); // Did we want to accept query only transactions?
        let (btx, class) = tx.into_blockifier(
            self.backend.chain_config().chain_id.to_felt(),
            self.backend.chain_config().latest_protocol_version,
            validate,
            true,
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
        let validate = !tx.is_query(); // Did we want to accept query only transactions?
        let (btx, class) = tx.into_blockifier(
            self.backend.chain_config().chain_id.to_felt(),
            self.backend.chain_config().latest_protocol_version,
            validate,
            true,
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
        let validate = !tx.is_query(); // Did we want to accept query only transactions?
        let (btx, class) = tx.into_blockifier(
            self.backend.chain_config().chain_id.to_felt(),
            self.backend.chain_config().latest_protocol_version,
            validate,
            true,
        )?;

        let res = AddInvokeTransactionResult { transaction_hash: btx.tx_hash().to_felt() };
        self.accept_tx(btx, class, arrived_at).await?;
        Ok(res)
    }
}
