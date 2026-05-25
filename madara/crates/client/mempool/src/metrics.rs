use crate::inner::{limits::MempoolLimitReached, MempoolStateSummary, TxInsertionError};
use mc_telemetry::{register_counter_metric_instrument, register_gauge_metric_instrument};
use mp_transactions::{validated::ValidatedTransaction, Transaction};
use opentelemetry::metrics::{Counter, Gauge};
use opentelemetry::{global, InstrumentationScope, KeyValue};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MempoolIngressSource {
    Unknown,
    Rpc,
    Gateway,
    Reinsert,
    DbRestore,
}

impl MempoolIngressSource {
    pub fn as_label(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Rpc => "rpc",
            Self::Gateway => "gateway",
            Self::Reinsert => "reinsert",
            Self::DbRestore => "db_restore",
        }
    }

    pub fn should_record_ingress_metrics(self) -> bool {
        !matches!(self, Self::DbRestore)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MempoolAddRejectReason {
    DuplicateTxn,
    InvalidContractAddress,
    InvalidNonce,
    Internal,
    MaxDeclareTransactions,
    MaxTransactions,
    MinTipBump,
    NonceConflict,
    NonceTooLow,
    PendingDeclare,
    TooOld,
    ValidatedToBlockifier,
}

impl MempoolAddRejectReason {
    pub fn from_inner_error(inner: &TxInsertionError) -> Self {
        match inner {
            TxInsertionError::NonceTooLow { .. } => Self::NonceTooLow,
            TxInsertionError::NonceConflict => Self::NonceConflict,
            TxInsertionError::DuplicateTxn => Self::DuplicateTxn,
            TxInsertionError::MinTipBump { .. } => Self::MinTipBump,
            TxInsertionError::TooOld { .. } => Self::TooOld,
            TxInsertionError::PendingDeclare => Self::PendingDeclare,
            TxInsertionError::InvalidContractAddress => Self::InvalidContractAddress,
            TxInsertionError::Limit(limit) => match limit {
                MempoolLimitReached::MaxTransactions { .. } => Self::MaxTransactions,
                MempoolLimitReached::MaxDeclareTransactions { .. } => Self::MaxDeclareTransactions,
            },
        }
    }

    pub fn as_label(self) -> &'static str {
        match self {
            Self::DuplicateTxn => "duplicate_txn",
            Self::InvalidContractAddress => "invalid_contract_address",
            Self::InvalidNonce => "invalid_nonce",
            Self::Internal => "internal",
            Self::MaxDeclareTransactions => "max_declare_transactions",
            Self::MaxTransactions => "max_transactions",
            Self::MinTipBump => "min_tip_bump",
            Self::NonceConflict => "nonce_conflict",
            Self::NonceTooLow => "nonce_too_low",
            Self::PendingDeclare => "pending_declare",
            Self::TooOld => "too_old",
            Self::ValidatedToBlockifier => "validated_to_blockifier",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MempoolRemovalReason {
    AdminFlush,
    ConsumedForBlock,
    DisplacedByInsert,
    NonceAdvanced,
    TtlExpired,
}

impl MempoolRemovalReason {
    pub fn as_label(self) -> &'static str {
        match self {
            Self::AdminFlush => "admin_flush",
            Self::ConsumedForBlock => "consumed_for_block",
            Self::DisplacedByInsert => "displaced_by_insert",
            Self::NonceAdvanced => "nonce_advanced",
            Self::TtlExpired => "ttl_expired",
        }
    }
}

fn tx_type_label(tx: &ValidatedTransaction) -> &'static str {
    match &tx.transaction {
        Transaction::Invoke(_) => "invoke",
        Transaction::L1Handler(_) => "l1_handler",
        Transaction::Declare(_) => "declare",
        Transaction::Deploy(_) => "deploy",
        Transaction::DeployAccount(_) => "deploy_account",
    }
}

#[derive(Clone)]
pub struct MempoolMetrics {
    pub accepted_transaction_counter: Counter<u64>,
    pub mempool_current_size: Gauge<u64>,
    pub mempool_ready_transactions: Gauge<u64>,
    pub mempool_transactions_current: Gauge<u64>,
    pub mempool_capacity_transactions: Gauge<u64>,
    pub mempool_accounts_current: Gauge<u64>,
    pub mempool_ready_transactions_current: Gauge<u64>,
    pub mempool_queued_transactions_current: Gauge<u64>,
    pub mempool_oldest_transaction_age_seconds: Gauge<f64>,
    pub mempool_oldest_ready_transaction_age_seconds: Gauge<f64>,
    pub mempool_add_attempts_total: Counter<u64>,
    pub mempool_add_success_total: Counter<u64>,
    pub mempool_add_rejected_total: Counter<u64>,
    pub mempool_removed_total: Counter<u64>,
}

impl MempoolMetrics {
    pub fn register() -> Self {
        // Register meter
        let meter = global::meter_with_scope(
            InstrumentationScope::builder("crates.mempool.opentelemetry")
                .with_attributes([KeyValue::new("crate", "mempool")])
                .build(),
        );

        let accepted_transaction_counter = register_counter_metric_instrument(
            &meter,
            "accepted_transaction_count".to_string(),
            "A counter to show accepted transactions in the mempool".to_string(),
            "transaction".to_string(),
        );

        let mempool_current_size = register_gauge_metric_instrument(
            &meter,
            "mempool_current_size".to_string(),
            "Current number of transactions in the mempool".to_string(),
            "transaction".to_string(),
        );

        let mempool_ready_transactions = register_gauge_metric_instrument(
            &meter,
            "mempool_ready_transactions".to_string(),
            "Number of ready transactions in the mempool".to_string(),
            "transaction".to_string(),
        );

        let mempool_transactions_current = register_gauge_metric_instrument(
            &meter,
            "mempool_transactions_current".to_string(),
            "Current number of transactions in the mempool".to_string(),
            "transaction".to_string(),
        );

        let mempool_capacity_transactions = register_gauge_metric_instrument(
            &meter,
            "mempool_capacity_transactions".to_string(),
            "Configured transaction capacity of the mempool".to_string(),
            "transaction".to_string(),
        );

        let mempool_accounts_current = register_gauge_metric_instrument(
            &meter,
            "mempool_accounts_current".to_string(),
            "Current number of accounts represented in the mempool".to_string(),
            "account".to_string(),
        );

        let mempool_ready_transactions_current = register_gauge_metric_instrument(
            &meter,
            "mempool_ready_transactions_current".to_string(),
            "Current number of ready transactions in the mempool".to_string(),
            "transaction".to_string(),
        );

        let mempool_queued_transactions_current = register_gauge_metric_instrument(
            &meter,
            "mempool_queued_transactions_current".to_string(),
            "Current number of queued non-ready transactions in the mempool".to_string(),
            "transaction".to_string(),
        );

        let mempool_oldest_transaction_age_seconds = register_gauge_metric_instrument(
            &meter,
            "mempool_oldest_transaction_age_seconds".to_string(),
            "Age of the oldest transaction currently in the mempool".to_string(),
            "s".to_string(),
        );

        let mempool_oldest_ready_transaction_age_seconds = register_gauge_metric_instrument(
            &meter,
            "mempool_oldest_ready_transaction_age_seconds".to_string(),
            "Age of the oldest ready transaction currently in the mempool".to_string(),
            "s".to_string(),
        );

        let mempool_add_attempts_total = register_counter_metric_instrument(
            &meter,
            "mempool_add_attempts_total".to_string(),
            "Attempts to add validated transactions to the mempool".to_string(),
            "transaction".to_string(),
        );

        let mempool_add_success_total = register_counter_metric_instrument(
            &meter,
            "mempool_add_success_total".to_string(),
            "Validated transactions successfully added to the mempool".to_string(),
            "transaction".to_string(),
        );

        let mempool_add_rejected_total = register_counter_metric_instrument(
            &meter,
            "mempool_add_rejected_total".to_string(),
            "Validated transactions rejected by the mempool".to_string(),
            "transaction".to_string(),
        );

        let mempool_removed_total = register_counter_metric_instrument(
            &meter,
            "mempool_removed_total".to_string(),
            "Transactions removed from the mempool".to_string(),
            "transaction".to_string(),
        );

        Self {
            accepted_transaction_counter,
            mempool_current_size,
            mempool_ready_transactions,
            mempool_transactions_current,
            mempool_capacity_transactions,
            mempool_accounts_current,
            mempool_ready_transactions_current,
            mempool_queued_transactions_current,
            mempool_oldest_transaction_age_seconds,
            mempool_oldest_ready_transaction_age_seconds,
            mempool_add_attempts_total,
            mempool_add_success_total,
            mempool_add_rejected_total,
            mempool_removed_total,
        }
    }

    pub fn record_mempool_state(&self, summary: &MempoolStateSummary) {
        self.mempool_current_size.record(summary.num_transactions as u64, &[]);
        self.mempool_ready_transactions.record(summary.ready_transactions as u64, &[]);
        self.mempool_transactions_current.record(summary.num_transactions as u64, &[]);
        self.mempool_capacity_transactions.record(summary.transaction_capacity as u64, &[]);
        self.mempool_accounts_current.record(summary.num_accounts as u64, &[]);
        self.mempool_ready_transactions_current.record(summary.ready_transactions as u64, &[]);
        self.mempool_queued_transactions_current.record(summary.queued_transactions as u64, &[]);
        self.mempool_oldest_transaction_age_seconds
            .record(summary.oldest_transaction_age.map(|age| age.as_secs_f64()).unwrap_or_default(), &[]);
        self.mempool_oldest_ready_transaction_age_seconds
            .record(summary.oldest_ready_transaction_age.map(|age| age.as_secs_f64()).unwrap_or_default(), &[]);

        #[cfg(test)]
        if test_counters::metrics_enabled() {
            test_counters::MEMPOOL_TRANSACTIONS_CURRENT_LAST
                .store(summary.num_transactions as u64, std::sync::atomic::Ordering::Relaxed);
            test_counters::MEMPOOL_CAPACITY_TRANSACTIONS_LAST
                .store(summary.transaction_capacity as u64, std::sync::atomic::Ordering::Relaxed);
            test_counters::MEMPOOL_ACCOUNTS_CURRENT_LAST
                .store(summary.num_accounts as u64, std::sync::atomic::Ordering::Relaxed);
            test_counters::MEMPOOL_READY_TRANSACTIONS_CURRENT_LAST
                .store(summary.ready_transactions as u64, std::sync::atomic::Ordering::Relaxed);
            test_counters::MEMPOOL_QUEUED_TRANSACTIONS_CURRENT_LAST
                .store(summary.queued_transactions as u64, std::sync::atomic::Ordering::Relaxed);
            test_counters::MEMPOOL_OLDEST_TRANSACTION_AGE_MS_LAST.store(
                summary.oldest_transaction_age.map(|age| age.as_millis() as u64).unwrap_or_default(),
                std::sync::atomic::Ordering::Relaxed,
            );
            test_counters::MEMPOOL_OLDEST_READY_TRANSACTION_AGE_MS_LAST.store(
                summary.oldest_ready_transaction_age.map(|age| age.as_millis() as u64).unwrap_or_default(),
                std::sync::atomic::Ordering::Relaxed,
            );
        }
    }

    pub fn record_add_attempt(&self, source: MempoolIngressSource, tx: &ValidatedTransaction) {
        let tx_type = tx_type_label(tx);
        self.mempool_add_attempts_total
            .add(1, &[KeyValue::new("source", source.as_label()), KeyValue::new("tx_type", tx_type)]);

        #[cfg(test)]
        if test_counters::metrics_enabled() {
            test_counters::ADD_ATTEMPTS.lock().unwrap_or_else(|e| e.into_inner()).push((source, tx_type));
        }
    }

    pub fn record_add_success(&self, source: MempoolIngressSource, tx: &ValidatedTransaction) {
        let tx_type = tx_type_label(tx);
        self.mempool_add_success_total
            .add(1, &[KeyValue::new("source", source.as_label()), KeyValue::new("tx_type", tx_type)]);

        #[cfg(test)]
        if test_counters::metrics_enabled() {
            test_counters::ADD_SUCCESSES.lock().unwrap_or_else(|e| e.into_inner()).push((source, tx_type));
        }
    }

    pub fn record_add_rejected(
        &self,
        source: MempoolIngressSource,
        tx: &ValidatedTransaction,
        reason: MempoolAddRejectReason,
    ) {
        let tx_type = tx_type_label(tx);
        self.mempool_add_rejected_total.add(
            1,
            &[
                KeyValue::new("source", source.as_label()),
                KeyValue::new("tx_type", tx_type),
                KeyValue::new("reason", reason.as_label()),
            ],
        );

        #[cfg(test)]
        if test_counters::metrics_enabled() {
            test_counters::ADD_REJECTIONS.lock().unwrap_or_else(|e| e.into_inner()).push((source, tx_type, reason));
        }
    }

    pub fn record_removed(&self, reason: MempoolRemovalReason, count: u64) {
        self.mempool_removed_total.add(count, &[KeyValue::new("reason", reason.as_label())]);

        #[cfg(test)]
        if test_counters::metrics_enabled() {
            test_counters::REMOVALS.lock().unwrap_or_else(|e| e.into_inner()).push((reason, count));
        }
    }
}

/// Metrics for the external-db outbox (WAL) append path.
///
/// These are emitted from the mempool acceptance path because the outbox append happens there.
/// They still use the `external_db_*` prefix since they are part of the external-db pipeline.
pub struct ExternalDbOutboxMetrics {
    pub outbox_writes: Counter<u64>,
    pub outbox_write_errors: Counter<u64>,
    pub outbox_strict_rejections: Counter<u64>,
    pub outbox_rollback_delete_errors: Counter<u64>,
}

impl ExternalDbOutboxMetrics {
    pub fn register() -> Self {
        let meter = global::meter("madara.external_db");

        Self {
            outbox_writes: meter
                .u64_counter("external_db_outbox_writes")
                .with_description("Outbox entries appended to RocksDB")
                .build(),
            // Keep name + description consistent with mc-external-db metrics.rs.
            outbox_write_errors: meter
                .u64_counter("external_db_outbox_write_errors")
                .with_description("Outbox write errors")
                .build(),
            outbox_strict_rejections: meter
                .u64_counter("external_db_outbox_strict_rejections")
                .with_description("Transactions rejected due to strict outbox write failure")
                .build(),
            outbox_rollback_delete_errors: meter
                .u64_counter("external_db_outbox_rollback_delete_errors")
                .with_description("Outbox rollback delete failures when mempool insertion fails")
                .build(),
        }
    }
}

#[cfg(test)]
pub mod test_counters {
    use super::{MempoolAddRejectReason, MempoolIngressSource, MempoolRemovalReason};
    use std::future::Future;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{LazyLock, Mutex};
    use tokio::sync::Mutex as AsyncMutex;

    static TEST_MUTEX: AsyncMutex<()> = AsyncMutex::const_new(());
    tokio::task_local! {
        static METRICS_ENABLED: ();
    }

    pub static ADD_ATTEMPTS: LazyLock<Mutex<Vec<(MempoolIngressSource, &'static str)>>> =
        LazyLock::new(|| Mutex::new(Vec::new()));
    pub static ADD_SUCCESSES: LazyLock<Mutex<Vec<(MempoolIngressSource, &'static str)>>> =
        LazyLock::new(|| Mutex::new(Vec::new()));
    pub static ADD_REJECTIONS: LazyLock<Mutex<Vec<(MempoolIngressSource, &'static str, MempoolAddRejectReason)>>> =
        LazyLock::new(|| Mutex::new(Vec::new()));
    pub static REMOVALS: LazyLock<Mutex<Vec<(MempoolRemovalReason, u64)>>> = LazyLock::new(|| Mutex::new(Vec::new()));

    pub static MEMPOOL_TRANSACTIONS_CURRENT_LAST: AtomicU64 = AtomicU64::new(0);
    pub static MEMPOOL_CAPACITY_TRANSACTIONS_LAST: AtomicU64 = AtomicU64::new(0);
    pub static MEMPOOL_ACCOUNTS_CURRENT_LAST: AtomicU64 = AtomicU64::new(0);
    pub static MEMPOOL_READY_TRANSACTIONS_CURRENT_LAST: AtomicU64 = AtomicU64::new(0);
    pub static MEMPOOL_QUEUED_TRANSACTIONS_CURRENT_LAST: AtomicU64 = AtomicU64::new(0);
    pub static MEMPOOL_OLDEST_TRANSACTION_AGE_MS_LAST: AtomicU64 = AtomicU64::new(0);
    pub static MEMPOOL_OLDEST_READY_TRANSACTION_AGE_MS_LAST: AtomicU64 = AtomicU64::new(0);

    pub async fn capture<T>(future: impl Future<Output = T>) -> T {
        let _guard = TEST_MUTEX.lock().await;
        reset_all();
        METRICS_ENABLED.scope((), future).await
    }

    pub fn metrics_enabled() -> bool {
        METRICS_ENABLED.try_with(|_| ()).is_ok()
    }

    pub fn reset_all() {
        ADD_ATTEMPTS.lock().unwrap_or_else(|e| e.into_inner()).clear();
        ADD_SUCCESSES.lock().unwrap_or_else(|e| e.into_inner()).clear();
        ADD_REJECTIONS.lock().unwrap_or_else(|e| e.into_inner()).clear();
        REMOVALS.lock().unwrap_or_else(|e| e.into_inner()).clear();

        MEMPOOL_TRANSACTIONS_CURRENT_LAST.store(0, Ordering::Relaxed);
        MEMPOOL_CAPACITY_TRANSACTIONS_LAST.store(0, Ordering::Relaxed);
        MEMPOOL_ACCOUNTS_CURRENT_LAST.store(0, Ordering::Relaxed);
        MEMPOOL_READY_TRANSACTIONS_CURRENT_LAST.store(0, Ordering::Relaxed);
        MEMPOOL_QUEUED_TRANSACTIONS_CURRENT_LAST.store(0, Ordering::Relaxed);
        MEMPOOL_OLDEST_TRANSACTION_AGE_MS_LAST.store(0, Ordering::Relaxed);
        MEMPOOL_OLDEST_READY_TRANSACTION_AGE_MS_LAST.store(0, Ordering::Relaxed);
    }
}
