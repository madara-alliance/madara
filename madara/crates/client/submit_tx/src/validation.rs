use crate::{
    RejectedTransactionError, RejectedTransactionErrorKind, SubmitTransaction, SubmitTransactionError,
    SubmitValidatedTransaction,
};
use async_trait::async_trait;
use blockifier::{
    blockifier::{stateful_validator::StatefulValidatorError, transaction_executor::TransactionExecutorError},
    state::errors::StateError,
    transaction::{
        account_transaction::{AccountTransaction, ExecutionFlags},
        errors::{TransactionExecutionError, TransactionPreValidationError},
        objects::HasRelatedFeeType,
    },
};
use mc_db::{MadaraBackend, MadaraBlockView};
use mc_exec::MadaraBlockViewExecutionExt;
use mc_mempool::{MempoolInsertionError, TxInsertionError};
use mp_class::ConvertedClass;
use mp_convert::ToFelt;
use mp_rpc::admin::BroadcastedDeclareTxnV0;
use mp_rpc::v0_9_0::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn,
    BroadcastedTxn, ClassAndTxnHash, ContractAndTxnHash,
};
use mp_transactions::{
    validated::{TxTimestamp, ValidatedTransaction},
    IntoStarknetApiExt, ToBlockifierError,
};
use starknet_api::{
    executable_transaction::{AccountTransaction as ApiAccountTransaction, TransactionType},
    transaction::TransactionVersion,
};
use starknet_types_core::felt::Felt;
use std::{borrow::Cow, fmt, sync::Arc};

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
            | E::PanicInValidate { .. }
            | E::DeclareTransactionCasmHashMissMatch { .. }
            | E::ValidateCairo0Error(_)) => rejected(ValidateFailure, format!("{err:#}")),
            err @ (E::FeeCheckError(_)
            | E::FromStr(_)
            | E::InvalidValidateReturnData { .. }
            | E::StarknetApiError(_)
            | E::TransactionFeeError(_)
            | E::TryFromIntError(_)
            | E::TransactionTooLarge { .. }) => rejected(ValidateFailure, format!("{err:#}")),
            err @ E::InvalidVersion { .. } => rejected(InvalidTransactionVersion, format!("{err:#}")),
            err @ (E::InvalidSegmentStructure(_, _) | E::ProgramError { .. }) => {
                rejected(InvalidProgram, format!("{err:#}"))
            }
            E::TransactionPreValidationError(err) => (*err).into(),
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
            E::UnsupportedProtocolVersion(_) | E::Internal(_) | E::InvalidSequencerAddress(_) => {
                Internal(anyhow::anyhow!(value))
            }
        }
    }
}

impl From<MempoolInsertionError> for SubmitTransactionError {
    fn from(value: MempoolInsertionError) -> Self {
        use MempoolInsertionError as E;
        use RejectedTransactionErrorKind::*;
        use SubmitTransactionError::*;

        fn rejected(
            kind: RejectedTransactionErrorKind,
            message: impl Into<Cow<'static, str>>,
        ) -> SubmitTransactionError {
            SubmitTransactionError::Rejected(RejectedTransactionError::new(kind, message))
        }

        match value {
            err @ (E::Internal(_) | E::ValidatedToBlockifier(_)) => Internal(anyhow::anyhow!(err)),
            err @ E::InnerMempool(TxInsertionError::TooOld { .. }) => Internal(anyhow::anyhow!(err)),
            E::InnerMempool(TxInsertionError::DuplicateTxn) => {
                rejected(DuplicatedTransaction, "A transaction with this hash already exists in the transaction pool")
            }
            E::InnerMempool(TxInsertionError::Limit(limit)) => rejected(TransactionLimitExceeded, format!("{limit:#}")),
            E::InnerMempool(TxInsertionError::NonceConflict) => rejected(
                InvalidTransactionNonce,
                "A transaction with this nonce already exists in the transaction pool",
            ),
            E::InnerMempool(TxInsertionError::PendingDeclare) => {
                rejected(InvalidTransactionNonce, "Cannot add a declare transaction with a future nonce")
            }
            E::InnerMempool(TxInsertionError::MinTipBump { min_tip_bump }) => rejected(
                ValidateFailure,
                format!("Replacing a transaction requires increasing the tip by at least {}%", min_tip_bump * 10.0),
            ),
            E::InnerMempool(TxInsertionError::InvalidContractAddress) => {
                rejected(ValidateFailure, "Invalid contract address")
            }
            E::InnerMempool(TxInsertionError::NonceTooLow { account_nonce }) => rejected(
                InvalidTransactionNonce,
                format!("Nonce needs to be greater than the account nonce {:#x}", account_nonce.to_felt()),
            ),
            E::InvalidNonce => rejected(InvalidTransactionNonce, "Invalid transaction nonce"),
        }
    }
}

#[derive(Debug, Default)]
pub struct TransactionValidatorConfig {
    pub disable_validation: bool,
    pub disable_fee: bool,
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

impl fmt::Debug for TransactionValidator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TransactionValidator {{ config: {:?} }}", self.config)
    }
}

impl TransactionValidator {
    pub fn new(
        inner: Arc<dyn SubmitValidatedTransaction>,
        backend: Arc<MadaraBackend>,
        config: TransactionValidatorConfig,
    ) -> Self {
        Self { inner, backend, config }
    }

    #[tracing::instrument(skip(self, tx, converted_class))]
    async fn accept_tx(
        &self,
        tx: ApiAccountTransaction,
        converted_class: Option<ConvertedClass>,
        arrived_at: TxTimestamp,
    ) -> Result<(), SubmitTransactionError> {
        let tx_hash = tx.tx_hash().to_felt();

        if tx.tx_type() == TransactionType::L1Handler {
            // L1HandlerTransactions don't have nonces.
            return Err(RejectedTransactionError::new(
                RejectedTransactionErrorKind::ValidateFailure,
                "Cannot submit l1 handler transactions",
            )
            .into());
        };
        // We have to skip part of the validation in the very specific case where you send an invoke tx directly after a deploy account:
        // the account is not deployed yet but the tx should be accepted.
        let validate = !(tx.tx_type() == TransactionType::InvokeFunction && tx.nonce().to_felt() == Felt::ONE);

        // No charge_fee for Admin DeclareV0
        let charge_fee = !((tx.tx_type() == TransactionType::Declare
            && tx.version() == TransactionVersion(Felt::ZERO))
            || self.config.disable_fee);

        let account_tx = AccountTransaction {
            tx,
            execution_flags: ExecutionFlags { only_query: false, charge_fee, validate, strict_nonce_check: false },
        };

        if !self.config.disable_validation {
            if account_tx.version() < TransactionVersion::ONE {
                // Some v0 txs don't have a nonce. (declare)
                return Err(RejectedTransactionError::new(
                    RejectedTransactionErrorKind::InvalidTransactionVersion,
                    "Cannot submit v0 transaction",
                )
                .into());
            };

            tracing::debug!("Mempool verify tx_hash={:#x}", tx_hash);
            // Perform validations
            let mut validator = MadaraBlockView::from(self.backend.block_view_on_preconfirmed_or_fake()?)
                .new_execution_context()?
                .into_transaction_validator();
            validator.perform_validations(account_tx.clone())?
        }

        // Forward the validated tx.
        let tx = ValidatedTransaction::from_starknet_api(account_tx.tx, arrived_at, converted_class);
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
        if tx.is_query() {
            return Err(RejectedTransactionError::new(
                RejectedTransactionErrorKind::InvalidTransactionVersion,
                "Cannot submit query-only transactions",
            )
            .into());
        }

        let arrived_at = TxTimestamp::now();

        let (api_tx, class) = tx.into_starknet_api(
            self.backend.chain_config().chain_id.to_felt(),
            self.backend.chain_config().latest_protocol_version,
        )?;

        // Destructure to get class hash only if it's a Declare tx
        let class_hash = match &api_tx {
            ApiAccountTransaction::Declare(declare_tx) => declare_tx.class_hash().to_felt(),
            _ => unreachable!("Created transaction should be Declare"),
        };

        let res = ClassAndTxnHash { transaction_hash: api_tx.tx_hash().to_felt(), class_hash };

        self.accept_tx(api_tx, class, arrived_at).await?;
        Ok(res)
    }

    async fn submit_declare_transaction(
        &self,
        tx: BroadcastedDeclareTxn,
    ) -> Result<ClassAndTxnHash, SubmitTransactionError> {
        if tx.is_query() {
            return Err(RejectedTransactionError::new(
                RejectedTransactionErrorKind::InvalidTransactionVersion,
                "Cannot submit query-only transactions",
            )
            .into());
        }

        let arrived_at = TxTimestamp::now();
        let tx: BroadcastedTxn = BroadcastedTxn::Declare(tx);
        let (api_tx, class) = tx.into_starknet_api(
            self.backend.chain_config().chain_id.to_felt(),
            self.backend.chain_config().latest_protocol_version,
        )?;

        // Destructure to get class hash only if it's a Declare tx
        let class_hash = match &api_tx {
            ApiAccountTransaction::Declare(declare_tx) => declare_tx.class_hash().to_felt(),
            _ => unreachable!("Created transaction should be Declare"),
        };

        let res = ClassAndTxnHash { transaction_hash: api_tx.tx_hash().to_felt(), class_hash };

        self.accept_tx(api_tx, class, arrived_at).await?;
        Ok(res)
    }

    async fn submit_deploy_account_transaction(
        &self,
        tx: BroadcastedDeployAccountTxn,
    ) -> Result<ContractAndTxnHash, SubmitTransactionError> {
        if tx.is_query() {
            return Err(RejectedTransactionError::new(
                RejectedTransactionErrorKind::InvalidTransactionVersion,
                "Cannot submit query-only transactions",
            )
            .into());
        }

        let arrived_at = TxTimestamp::now();
        let tx = BroadcastedTxn::DeployAccount(tx);
        let (api_tx, class) = tx.into_starknet_api(
            self.backend.chain_config().chain_id.to_felt(),
            self.backend.chain_config().latest_protocol_version,
        )?;

        // Destructure to get class hash only if it's a DeployAccount tx
        let contract_address = match &api_tx {
            ApiAccountTransaction::DeployAccount(deploy_account_tx) => deploy_account_tx.contract_address().to_felt(),
            _ => unreachable!("Created transaction should be DeployAccount"),
        };

        let res = ContractAndTxnHash { transaction_hash: api_tx.tx_hash().to_felt(), contract_address };

        self.accept_tx(api_tx, class, arrived_at).await?;
        Ok(res)
    }

    async fn submit_invoke_transaction(
        &self,
        tx: BroadcastedInvokeTxn,
    ) -> Result<AddInvokeTransactionResult, SubmitTransactionError> {
        if tx.is_query() {
            return Err(RejectedTransactionError::new(
                RejectedTransactionErrorKind::InvalidTransactionVersion,
                "Cannot submit query-only transactions",
            )
            .into());
        }

        let arrived_at = TxTimestamp::now();
        let tx = BroadcastedTxn::Invoke(tx);
        let (api_tx, class) = tx.into_starknet_api(
            self.backend.chain_config().chain_id.to_felt(),
            self.backend.chain_config().latest_protocol_version,
        )?;

        let res = AddInvokeTransactionResult { transaction_hash: api_tx.tx_hash().to_felt() };
        self.accept_tx(api_tx, class, arrived_at).await?;
        Ok(res)
    }

    async fn received_transaction(&self, hash: mp_convert::Felt) -> Option<bool> {
        self.inner.received_transaction(hash).await
    }

    async fn subscribe_new_transactions(&self) -> Option<tokio::sync::broadcast::Receiver<mp_convert::Felt>> {
        self.inner.subscribe_new_transactions().await
    }
}
