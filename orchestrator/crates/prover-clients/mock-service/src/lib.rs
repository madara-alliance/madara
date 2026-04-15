//! Mock prover service for mocknet / dev environments.
//!
//! Does not submit proofs anywhere. Optionally registers the aggregator fact hash on an
//! L1 `MockGpsVerifier` contract so the settlement core contract's `isValid(fact)` check
//! passes on state-transition txs. The fact hash itself is threaded through:
//!
//! - `submit_task`: receives `fact` via [`ApplicativeJobInfo::fact_hash`], delegates to
//!   [`FactRegistrar::ensure_registered`] (idempotent, waits for receipt, cross-verifies),
//!   returns the tx hash — or `B256::ZERO` when no tx was needed.
//! - `get_task_status`: receives the fact (hex) via the trait's `fact` parameter and
//!   verifies via `isValid`. The `task_id` is intentionally unused on the aggregation path.

use alloy::primitives::{Address, B256};
use alloy::signers::local::PrivateKeySigner;
use async_trait::async_trait;
use orchestrator_gps_fact_checker::{FactCheckerError, FactRegistrar};
use orchestrator_prover_client_interface::{
    AggregationArtifacts, ApplicativeJobInfo, ProverClient, ProverClientError, Task, TaskStatus, TaskType,
};
use url::Url;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct MockValidatedArgs {
    /// When `Some`, the aggregator fact hash is registered on this contract during
    /// `submit_task(RunAggregationWithPie)`. When `None`, fact registration is skipped
    /// (assume the settlement core contract points at an always-true verifier stub).
    pub verifier_address: Option<Address>,
    pub ethereum_rpc_url: Url,
    /// Pre-parsed signer. Parsed once at CLI validation time (`TryFrom<RunCmd> for
    /// ProverConfig`), so `FactRegistrar::new` can stay infallible. `PrivateKeySigner`'s
    /// `Debug` impl redacts the secret.
    pub ethereum_signer: PrivateKeySigner,
}

pub struct MockProverService {
    /// Built iff `verifier_address` is configured. When `None`, the mock prover is a
    /// true no-op: no tx submission, no `isValid` check.
    fact_registrar: Option<FactRegistrar>,
}

impl MockProverService {
    pub fn new_with_args(args: &MockValidatedArgs) -> Self {
        let fact_registrar = args
            .verifier_address
            .map(|addr| FactRegistrar::new(args.ethereum_rpc_url.clone(), args.ethereum_signer.clone(), addr));
        Self { fact_registrar }
    }
}

fn parse_fact_hex(fact_hex: &str) -> Result<[u8; 32], ProverClientError> {
    let stripped = fact_hex.strip_prefix("0x").unwrap_or(fact_hex);
    let bytes = hex::decode(stripped)
        .map_err(|e| ProverClientError::FailedToConvertFact(format!("invalid fact hex {fact_hex}: {e}")))?;
    bytes.try_into().map_err(|_| ProverClientError::FailedToConvertFact(format!("fact hex {fact_hex} is not 32 bytes")))
}

fn registrar_err(e: FactCheckerError) -> ProverClientError {
    ProverClientError::Internal(Box::new(e))
}

#[async_trait]
impl ProverClient for MockProverService {
    #[tracing::instrument(skip(self, task), ret, err)]
    async fn submit_task(&self, task: Task) -> Result<String, ProverClientError> {
        match task {
            // Child proving jobs are a no-op under Mock: return a synthetic id that
            // `get_task_status(Job, ..)` unconditionally reports as Succeeded.
            Task::CreateJob(_) => Ok(Uuid::new_v4().to_string()),

            // Mock does not maintain remote bucket state; any unique id is fine.
            Task::CreateBucket => Ok(Uuid::new_v4().to_string()),

            Task::RunAggregation(_) => Err(ProverClientError::TaskInvalid(
                "Mock prover does not support bucket-based aggregation; use RunAggregationWithPie.".to_string(),
            )),

            Task::RunAggregationWithPie(ApplicativeJobInfo { fact_hash, .. }) => {
                match (&self.fact_registrar, fact_hash) {
                    (Some(registrar), Some(fact)) => {
                        let tx_hash = registrar.ensure_registered(fact).await.map_err(registrar_err)?;
                        tracing::info!(
                            tx_hash = %tx_hash,
                            fact = %hex::encode(fact),
                            "Mock: fact registered on-chain"
                        );
                        Ok(format!("{:#x}", tx_hash))
                    }
                    // No verifier configured or no fact produced — nothing to register.
                    _ => Ok(format!("{:#x}", B256::ZERO)),
                }
            }
        }
    }

    #[tracing::instrument(skip(self, fact), ret, err)]
    async fn get_task_status(
        &self,
        task: TaskType,
        _task_id: &str,
        fact: Option<String>,
        _cross_verify: bool,
    ) -> Result<TaskStatus, ProverClientError> {
        match task {
            TaskType::Job => Ok(TaskStatus::Succeeded),
            TaskType::Aggregation => match (&self.fact_registrar, fact) {
                (Some(registrar), Some(fact_hex)) => {
                    let fact_bytes = parse_fact_hex(&fact_hex)?;
                    if registrar.is_registered(fact_bytes).await.map_err(registrar_err)? {
                        Ok(TaskStatus::Succeeded)
                    } else {
                        Ok(TaskStatus::Failed(format!("fact {fact_hex} not registered on mock verifier")))
                    }
                }
                // No verifier configured, or no fact supplied to cross-check — nothing to do.
                _ => Ok(TaskStatus::Succeeded),
            },
        }
    }

    async fn get_proof(&self, _task_id: &str) -> Result<String, ProverClientError> {
        Err(ProverClientError::TaskInvalid("Mock prover does not produce proofs".to_string()))
    }

    async fn submit_l2_query(
        &self,
        _task_id: &str,
        _fact: &str,
        _n_steps: Option<usize>,
    ) -> Result<String, ProverClientError> {
        Err(ProverClientError::TaskInvalid("Mock prover does not support L2 queries".to_string()))
    }

    /// Mock aggregates locally in the handler; artifacts are already written to storage
    /// before this point, so we have nothing to return.
    async fn get_aggregation_artifacts(
        &self,
        _external_id: &str,
        _include_proof: bool,
    ) -> Result<AggregationArtifacts, ProverClientError> {
        Ok(AggregationArtifacts::default())
    }
}
