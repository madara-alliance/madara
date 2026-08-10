use crate::batcher::Batcher;
use crate::close_queue::{CloseJobCompletion, QueuedClosePayload};
use crate::comparator::{
    decide, execution_resources::compare_execution_resources,
    state_diff::compare_state_diff_with_ignored_storage_addresses, CanonicalBlockSource, CanonicalizedBlockOutput,
    ComparatorDecision, ExecutionResourceComparison,
};
use crate::executor::{self, BatchExecutionResult, ExecutorMessage};
use crate::fallback::manager::FallbackManager;
use crate::fallback::types::{ExecutionMode, FallbackReason, StartupExecutionMode};
use crate::finalizer::FinalizerHandle;
use crate::handle::BlockProductionHandle;
use crate::metrics::BlockProductionMetrics;
use crate::reexecution::dispatcher::{new_epoch_token, start_reexec_dispatcher, ReexecDispatcherHandle};
use crate::reexecution::{ReexecExecutedTxArtifacts, ReexecRequest, ReexecWorkerOutcome};
use crate::util;
use crate::util::{AdditionalTxInfo, BatchToExecute};
use anyhow::Context;
use blockifier::blockifier::transaction_executor::BlockExecutionSummary;
use mc_db::close_pipeline_contract::ClosePreconfirmedResult;
use mc_db::preconfirmed::{PreconfirmedBlock, PreconfirmedExecutedTransaction};
use mc_db::{MadaraBackend, MadaraPreconfirmedBlockView, MadaraStateView, MadaraStorageRead};
use mc_exec::execution::TxInfo;
use mc_exec::LayeredStateAdapter;
use mc_mempool::Mempool;
use mc_settlement_client::SettlementClient;
use mp_block::{header::PreconfirmedHeader, TransactionWithReceipt};
use mp_chain_config::RuntimeExecutionConfig;
use mp_convert::{Felt, ToFelt};
use mp_receipt::from_blockifier_execution_info;
use mp_state_update::StateDiff;
use mp_state_update::{ClassUpdateItem, DeclaredClassCompiledClass, TransactionStateUpdate};
use mp_transactions::validated::ValidatedTransaction;
use mp_transactions::TransactionWithHash;
use mp_utils::rayon::global_spawn_rayon_task;
use mp_utils::service::ServiceContext;
use mp_utils::AbortOnDrop;
use opentelemetry::KeyValue;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::mem;
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};
use tokio::sync::mpsc;
use tokio::sync::watch;
use tokio::task::JoinHandle;

mod canonicalization;
mod close_pipeline;
mod run_loop;
mod startup_recovery;
mod state;
mod tainted_rebuild;
#[cfg(test)]
pub(crate) mod tests;

use self::state::{
    ApprovedExternalContent, CanonicalizationTaskCanonical, CanonicalizationTaskResult, PendingCanonicalizationInput,
    PendingStopFallbackHandoff, PendingTaintedRebuildCarry, StartupRecoveredBlockPayload, TaintedRebuildClosePayload,
    TaintedRebuildSourceTx, TaintedRebuildStepResult,
};
pub use self::state::{BlockProductionStateNotification, BlockProductionTask};
pub(crate) use self::state::{CurrentBlockState, MempoolIntakeMode, TaskState};
