//! Outbox drainer and batch writer for external database.

use crate::config::ExternalDbConfig;
use crate::metrics::ExternalDbMetrics;
use crate::mongodb::{indexes::get_index_models, MongoClient, MempoolTransactionDocument, ResourceBoundsDoc};
use anyhow::Context;
use mc_db::MadaraBackend;
use mp_convert::FeltHexDisplay;
use mp_transactions::{
    validated::ValidatedTransaction, DeclareTransaction, DeployAccountTransaction, DeployTransaction, InvokeTransaction,
    Transaction,
};
use mp_utils::service::ServiceContext;
use mongodb::{bson, error::ErrorKind, options::InsertManyOptions};
use std::sync::Arc;
use std::time::{Duration, Instant};

const STATUS_PENDING: &str = "PENDING";

#[async_trait::async_trait]
trait ExternalDbSink: Send + Sync {
    async fn ensure_indexes(&self) -> anyhow::Result<()>;
    async fn insert_many(&self, docs: Vec<MempoolTransactionDocument>) -> anyhow::Result<InsertManySummary>;
    async fn delete_by_hashes(&self, hashes: Vec<String>) -> anyhow::Result<u64>;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct InsertManySummary {
    inserted: usize,
    duplicates: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct DrainSummary {
    attempted: usize,
    inserted: usize,
    duplicates: usize,
}

struct MongoSink {
    collection: mongodb::Collection<MempoolTransactionDocument>,
}

#[async_trait::async_trait]
impl ExternalDbSink for MongoSink {
    async fn ensure_indexes(&self) -> anyhow::Result<()> {
        let indexes = get_index_models();
        self.collection.create_indexes(indexes).await?;
        Ok(())
    }

    async fn insert_many(&self, docs: Vec<MempoolTransactionDocument>) -> anyhow::Result<InsertManySummary> {
        if docs.is_empty() {
            return Ok(InsertManySummary::default());
        }

        let total = docs.len();
        let options = InsertManyOptions::builder().ordered(false).build();
        match self.collection.insert_many(docs).with_options(options).await {
            Ok(result) => Ok(InsertManySummary { inserted: result.inserted_ids.len(), duplicates: 0 }),
            Err(err) => match classify_duplicate_only_error(&err) {
                Some(duplicates) => Ok(InsertManySummary { inserted: total.saturating_sub(duplicates), duplicates }),
                None => Err(err.into()),
            },
        }
    }

    async fn delete_by_hashes(&self, hashes: Vec<String>) -> anyhow::Result<u64> {
        if hashes.is_empty() {
            return Ok(0);
        }

        let filter = bson::doc! { "_id": { "$in": hashes } };
        let result = self.collection.delete_many(filter).await?;
        Ok(result.deleted_count)
    }
}

fn classify_duplicate_only_error(err: &mongodb::error::Error) -> Option<usize> {
    let ErrorKind::InsertMany(failure) = &*err.kind else { return None };
    if failure.write_concern_error.is_some() {
        return None;
    }
    let write_errors = failure.write_errors.as_ref()?;
    if write_errors.is_empty() {
        return None;
    }

    let mut duplicates = 0usize;
    for write_error in write_errors {
        if write_error.code == 11000 {
            duplicates += 1;
        } else {
            return None;
        }
    }
    Some(duplicates)
}

#[derive(Debug)]
struct RetryBackoff {
    base: Duration,
    max: Duration,
    current: Option<Duration>,
}

impl RetryBackoff {
    fn new(base: Duration, max: Duration) -> Self {
        Self { base, max, current: None }
    }

    fn next_delay(&mut self) -> Duration {
        let next = match self.current {
            None => self.base,
            Some(current) => std::cmp::min(self.max, current.saturating_mul(2)),
        };
        self.current = Some(next);
        next
    }

    fn reset(&mut self) {
        self.current = None;
    }
}

#[derive(Debug, Clone)]
struct L1Confirmation {
    block_number: u64,
    tx_hashes: Vec<mp_convert::Felt>,
}

#[async_trait::async_trait]
trait L1ConfirmationSource: Send + Sync {
    async fn poll_confirmations(&mut self) -> anyhow::Result<Vec<L1Confirmation>>;
}

struct BackendL1ConfirmationSource {
    backend: Arc<MadaraBackend>,
    last_confirmed: Option<u64>,
}

impl BackendL1ConfirmationSource {
    fn new(backend: Arc<MadaraBackend>) -> Self {
        Self { backend, last_confirmed: None }
    }
}

#[async_trait::async_trait]
impl L1ConfirmationSource for BackendL1ConfirmationSource {
    async fn poll_confirmations(&mut self) -> anyhow::Result<Vec<L1Confirmation>> {
        let latest = self.backend.latest_l1_confirmed_block_n();
        let Some(latest) = latest else { return Ok(Vec::new()) };

        let start = self.last_confirmed.map(|n| n + 1).unwrap_or(0);
        if start > latest {
            return Ok(Vec::new());
        }

        let mut confirmations = Vec::new();
        for block_number in start..=latest {
            let Some(view) = self.backend.block_view_on_confirmed(block_number) else {
                continue;
            };
            let info = view.get_block_info().with_context(|| format!("Block info missing for block {block_number}"))?;
            confirmations.push(L1Confirmation { block_number, tx_hashes: info.tx_hashes });
        }

        self.last_confirmed = Some(latest);
        Ok(confirmations)
    }
}

#[derive(Debug, Clone)]
struct PendingDeletion {
    execute_at: Instant,
    tx_hashes: Vec<mp_convert::Felt>,
}

struct RetentionScheduler<S: L1ConfirmationSource> {
    source: S,
    delay: Duration,
    pending: std::collections::VecDeque<PendingDeletion>,
    metrics: Arc<ExternalDbMetrics>,
}

impl<S: L1ConfirmationSource> RetentionScheduler<S> {
    fn new(source: S, delay: Duration, metrics: Arc<ExternalDbMetrics>) -> Self {
        Self { source, delay, pending: std::collections::VecDeque::new(), metrics }
    }

    async fn tick(&mut self, now: Instant, sink: &dyn ExternalDbSink) -> anyhow::Result<()> {
        let confirmations = self.source.poll_confirmations().await?;
        for confirmation in confirmations {
            if confirmation.tx_hashes.is_empty() {
                continue;
            }
            tracing::debug!(
                "Scheduling external DB retention for block {} ({} txs)",
                confirmation.block_number,
                confirmation.tx_hashes.len()
            );
            self.pending.push_back(PendingDeletion {
                execute_at: now + self.delay,
                tx_hashes: confirmation.tx_hashes,
            });
        }

        while matches!(self.pending.front(), Some(pending) if pending.execute_at <= now) {
            let pending = self.pending.pop_front().expect("checked");
            let hashes = pending
                .tx_hashes
                .iter()
                .map(|felt| format!("{}", felt.hex_display()))
                .collect::<Vec<_>>();
            let deleted = sink.delete_by_hashes(hashes).await?;
            self.metrics.transactions_deleted.add(deleted as u64, &[]);
        }

        Ok(())
    }
}

/// Background worker that drains the outbox and writes to MongoDB.
pub struct ExternalDbWorker {
    config: ExternalDbConfig,
    backend: Arc<MadaraBackend>,
    chain_id: String,
    metrics: Arc<ExternalDbMetrics>,
}

impl ExternalDbWorker {
    /// Creates a new external database worker.
    pub fn new(
        config: ExternalDbConfig,
        backend: Arc<MadaraBackend>,
        chain_id: String,
        metrics: Arc<ExternalDbMetrics>,
    ) -> Self {
        Self { config, backend, chain_id, metrics }
    }

    /// Runs the worker loop until cancelled.
    pub async fn run(self, mut ctx: ServiceContext) -> anyhow::Result<()> {
        tracing::info!(
            "External DB worker started (database: {}, collection: {})",
            self.config.database_name,
            self.config.collection_name
        );

        let mongo = MongoClient::new(&self.config).await.context("Connecting to MongoDB")?;
        let sink = MongoSink { collection: mongo.collection().clone() };
        sink.ensure_indexes().await.context("Ensuring external DB indexes")?;
        self.startup_sync(&sink).await.context("External DB startup sync")?;

        let retention_delay = Duration::from_secs(self.config.retention_delay_secs);
        let mut retention = RetentionScheduler::new(
            BackendL1ConfirmationSource::new(self.backend.clone()),
            retention_delay,
            self.metrics.clone(),
        );

        let mut interval = tokio::time::interval(Duration::from_millis(self.config.flush_interval_ms));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut retention_interval = tokio::time::interval(Duration::from_secs(self.config.retention_tick_secs));
        retention_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        let mut retry_backoff = RetryBackoff::new(
            Duration::from_millis(self.config.retry_backoff_ms),
            Duration::from_millis(self.config.retry_backoff_max_ms),
        );
        let mut retry_at: Option<Instant> = None;

        loop {
            tokio::select! {
                _ = ctx.cancelled() => {
                    tracing::info!("External DB worker shutting down");
                    break;
                }
                _ = interval.tick() => {
                    if let Some(next_retry_at) = retry_at {
                        if Instant::now() < next_retry_at {
                            continue;
                        }
                    }

                    match self.drain_outbox_once(&sink).await {
                        Ok(summary) => {
                            if summary.attempted > 0 {
                                retry_backoff.reset();
                                retry_at = None;
                            }
                        }
                        Err(err) => {
                            self.metrics.mongodb_write_errors.add(1, &[]);
                            self.metrics.retry_count.add(1, &[]);
                            let delay = retry_backoff.next_delay();
                            retry_at = Some(Instant::now() + delay);
                            tracing::warn!("External DB batch write failed: {err:#}");
                        }
                    }
                }
                _ = retention_interval.tick() => {
                    if let Err(err) = retention.tick(Instant::now(), &sink).await {
                        tracing::warn!("External DB retention tick failed: {err:#}");
                    }
                }
            }
        }

        Ok(())
    }

    async fn drain_outbox_once(&self, sink: &dyn ExternalDbSink) -> anyhow::Result<DrainSummary> {
        let mut txs = Vec::new();
        for res in self.backend.get_external_outbox_transactions(self.config.batch_size) {
            txs.push(res.context("Reading external outbox transaction")?);
        }

        if txs.is_empty() {
            return Ok(DrainSummary::default());
        }

        let mut documents = Vec::with_capacity(txs.len());
        for tx in &txs {
            documents.push(self.convert_to_document(tx)?);
        }

        let start = Instant::now();
        let result = sink.insert_many(documents).await?;
        let elapsed = start.elapsed();
        self.metrics.write_latency.record(elapsed.as_secs_f64(), &[]);
        self.metrics.transactions_written.add(result.inserted as u64, &[]);

        for tx in &txs {
            self.backend.delete_external_outbox(tx.hash).context("Deleting external outbox transaction")?;
        }

        Ok(DrainSummary { attempted: txs.len(), inserted: result.inserted, duplicates: result.duplicates })
    }

    async fn startup_sync(&self, sink: &dyn ExternalDbSink) -> anyhow::Result<usize> {
        let mut total = 0;
        loop {
            let summary = self.drain_outbox_once(sink).await?;
            total += summary.attempted;
            if summary.attempted == 0 {
                break;
            }
        }
        Ok(total)
    }

    fn convert_to_document(&self, tx: &ValidatedTransaction) -> anyhow::Result<MempoolTransactionDocument> {
        let tx_type = match &tx.transaction {
            Transaction::Invoke(_) => "INVOKE",
            Transaction::Declare(_) => "DECLARE",
            Transaction::DeployAccount(_) => "DEPLOY_ACCOUNT",
            Transaction::L1Handler(_) => "L1_HANDLER",
            Transaction::Deploy(_) => "DEPLOY",
        }
        .to_string();

        let version = tx.transaction.version();
        let tx_version = if version == starknet_api::transaction::TransactionVersion::ZERO {
            "V0"
        } else if version == starknet_api::transaction::TransactionVersion::ONE {
            "V1"
        } else if version == starknet_api::transaction::TransactionVersion::TWO {
            "V2"
        } else if version == starknet_api::transaction::TransactionVersion::THREE {
            "V3"
        } else {
            "UNKNOWN"
        }
        .to_string();

        let sender_address = match &tx.transaction {
            Transaction::Invoke(t) => t.sender_address(),
            Transaction::Declare(t) => t.sender_address(),
            Transaction::DeployAccount(t) => t.sender_address(),
            Transaction::L1Handler(t) => &t.contract_address,
            Transaction::Deploy(_) => &tx.contract_address,
        };

        let signature = match &tx.transaction {
            Transaction::Invoke(t) => t.signature(),
            Transaction::Declare(t) => t.signature(),
            Transaction::DeployAccount(t) => t.signature(),
            Transaction::L1Handler(_) | Transaction::Deploy(_) => &[],
        };

        let calldata = match &tx.transaction {
            Transaction::Invoke(t) => Some(t.calldata().to_vec()),
            Transaction::DeployAccount(t) => Some(t.calldata().to_vec()),
            Transaction::L1Handler(t) => Some(t.calldata.as_ref().clone()),
            _ => None,
        };

        let class_hash = match &tx.transaction {
            Transaction::Declare(t) => Some(*t.class_hash()),
            Transaction::DeployAccount(t) => Some(match t {
                DeployAccountTransaction::V1(tx) => tx.class_hash,
                DeployAccountTransaction::V3(tx) => tx.class_hash,
            }),
            Transaction::Deploy(DeployTransaction { class_hash, .. }) => Some(*class_hash),
            _ => None,
        };

        let compiled_class_hash = match &tx.transaction {
            Transaction::Declare(DeclareTransaction::V2(tx)) => Some(tx.compiled_class_hash),
            Transaction::Declare(DeclareTransaction::V3(tx)) => Some(tx.compiled_class_hash),
            _ => None,
        };

        let constructor_calldata = match &tx.transaction {
            Transaction::DeployAccount(t) => Some(t.calldata().to_vec()),
            Transaction::Deploy(DeployTransaction { constructor_calldata, .. }) => Some(constructor_calldata.clone()),
            _ => None,
        };

        let contract_address_salt = match &tx.transaction {
            Transaction::DeployAccount(DeployAccountTransaction::V1(tx)) => Some(tx.contract_address_salt),
            Transaction::DeployAccount(DeployAccountTransaction::V3(tx)) => Some(tx.contract_address_salt),
            Transaction::Deploy(DeployTransaction { contract_address_salt, .. }) => Some(*contract_address_salt),
            _ => None,
        };

        let entry_point_selector = match &tx.transaction {
            Transaction::L1Handler(t) => Some(t.entry_point_selector),
            _ => None,
        };

        let max_fee = match &tx.transaction {
            Transaction::Invoke(InvokeTransaction::V0(tx)) => Some(tx.max_fee),
            Transaction::Invoke(InvokeTransaction::V1(tx)) => Some(tx.max_fee),
            Transaction::Declare(DeclareTransaction::V0(tx)) => Some(tx.max_fee),
            Transaction::Declare(DeclareTransaction::V1(tx)) => Some(tx.max_fee),
            Transaction::Declare(DeclareTransaction::V2(tx)) => Some(tx.max_fee),
            Transaction::DeployAccount(DeployAccountTransaction::V1(tx)) => Some(tx.max_fee),
            _ => None,
        };

        let (tip, resource_bounds) = match &tx.transaction {
            Transaction::Invoke(InvokeTransaction::V3(tx)) => (Some(tx.tip), Some(tx.resource_bounds.clone())),
            Transaction::Declare(DeclareTransaction::V3(tx)) => (Some(tx.tip), Some(tx.resource_bounds.clone())),
            Transaction::DeployAccount(DeployAccountTransaction::V3(tx)) => (Some(tx.tip), Some(tx.resource_bounds.clone())),
            _ => (None, None),
        };

        let resource_bounds_doc = resource_bounds.map(|bounds| {
            let (l1_data_gas_max_amount, l1_data_gas_max_price) = match bounds.l1_data_gas {
                Some(l1_data) => (
                    Some(l1_data.max_amount),
                    Some(mp_convert::hex_serde::u128_to_hex_string(l1_data.max_price_per_unit)),
                ),
                None => (None, None),
            };
            ResourceBoundsDoc {
                l1_gas_max_amount: bounds.l1_gas.max_amount,
                l1_gas_max_price: mp_convert::hex_serde::u128_to_hex_string(bounds.l1_gas.max_price_per_unit),
                l2_gas_max_amount: bounds.l2_gas.max_amount,
                l2_gas_max_price: mp_convert::hex_serde::u128_to_hex_string(bounds.l2_gas.max_price_per_unit),
                l1_data_gas_max_amount,
                l1_data_gas_max_price,
            }
        });

        let signature_hex = signature.iter().map(|felt| format!("{}", felt.hex_display())).collect::<Vec<_>>();
        let calldata_hex = calldata.map(|items| items.iter().map(|felt| format!("{}", felt.hex_display())).collect());

        let raw_bytes = bincode::serialize(tx).context("Serializing raw transaction")?;

        Ok(MempoolTransactionDocument {
            tx_hash: format!("{}", tx.hash.hex_display()),
            tx_type,
            tx_version,
            sender_address: format!("{}", sender_address.hex_display()),
            contract_address: format!("{}", tx.contract_address.hex_display()),
            nonce: format!("{}", tx.transaction.nonce().hex_display()),
            block_number: None,
            arrived_at: bson::DateTime::from_millis(tx.arrived_at.0 as i64),
            inserted_at: bson::DateTime::now(),
            max_fee: max_fee.map(|fee| format!("{}", fee.hex_display())),
            tip,
            resource_bounds: resource_bounds_doc,
            signature: signature_hex,
            calldata: calldata_hex,
            class_hash: class_hash.map(|hash| format!("{}", hash.hex_display())),
            compiled_class_hash: compiled_class_hash.map(|hash| format!("{}", hash.hex_display())),
            constructor_calldata: constructor_calldata.map(|items| items.iter().map(|felt| format!("{}", felt.hex_display())).collect()),
            contract_address_salt: contract_address_salt.map(|salt| format!("{}", salt.hex_display())),
            entry_point_selector: entry_point_selector.map(|selector| format!("{}", selector.hex_display())),
            paid_fee_on_l1: tx.paid_fee_on_l1.map(|fee| mp_convert::hex_serde::u128_to_hex_string(fee)),
            raw_transaction: bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: raw_bytes },
            chain_id: self.chain_id.clone(),
            status: STATUS_PENDING.to_string(),
            charge_fee: tx.charge_fee,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mp_convert::Felt;
    use mp_chain_config::ChainConfig;
    use starknet_api::{
        core::ContractAddress,
        executable_transaction::AccountTransaction,
        transaction::{InvokeTransaction, InvokeTransactionV3, TransactionHash},
    };
    use std::sync::Mutex;

    struct FakeSink {
        responses: Mutex<Vec<anyhow::Result<InsertManySummary>>>,
        calls: Mutex<Vec<Vec<MempoolTransactionDocument>>>,
        delete_calls: Mutex<Vec<Vec<String>>>,
    }

    impl FakeSink {
        fn new(responses: Vec<anyhow::Result<InsertManySummary>>) -> Self {
            Self {
                responses: Mutex::new(responses),
                calls: Mutex::new(Vec::new()),
                delete_calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl ExternalDbSink for FakeSink {
        async fn ensure_indexes(&self) -> anyhow::Result<()> {
            Ok(())
        }

        async fn insert_many(&self, docs: Vec<MempoolTransactionDocument>) -> anyhow::Result<InsertManySummary> {
            self.calls.lock().unwrap().push(docs);
            self.responses.lock().unwrap().remove(0)
        }

        async fn delete_by_hashes(&self, hashes: Vec<String>) -> anyhow::Result<u64> {
            let deleted = hashes.len() as u64;
            self.delete_calls.lock().unwrap().push(hashes);
            Ok(deleted)
        }
    }

    fn make_validated_tx(seed: u64) -> ValidatedTransaction {
        let tx_hash = TransactionHash(seed.into());
        let contract_address = Felt::from_hex_unchecked(
            "0x055be462e718c4166d656d11f89e341115b8bc82389c3762a10eade04fcb225d",
        );

        ValidatedTransaction::from_starknet_api(
            AccountTransaction::Invoke(starknet_api::executable_transaction::InvokeTransaction {
                tx: InvokeTransaction::V3(InvokeTransactionV3 {
                    sender_address: ContractAddress::try_from(contract_address).unwrap(),
                    resource_bounds: Default::default(),
                    tip: Default::default(),
                    signature: Default::default(),
                    nonce: Default::default(),
                    calldata: Default::default(),
                    nonce_data_availability_mode: Default::default(),
                    fee_data_availability_mode: Default::default(),
                    paymaster_data: Default::default(),
                    account_deployment_data: Default::default(),
                }),
                tx_hash,
            }),
            mp_transactions::validated::TxTimestamp::now(),
            None,
            true,
        )
    }

    #[tokio::test]
    async fn drain_outbox_inserts_to_mongo() {
        let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
        let metrics = Arc::new(ExternalDbMetrics::register());
        let config = ExternalDbConfig::new("mongodb://localhost:27017".to_string());
        let worker = ExternalDbWorker::new(config, backend.clone(), "MADARA_TEST".to_string(), metrics);

        let tx1 = make_validated_tx(1);
        let tx2 = make_validated_tx(2);
        backend.write_external_outbox(&tx1).unwrap();
        backend.write_external_outbox(&tx2).unwrap();

        let sink = FakeSink::new(vec![Ok(InsertManySummary { inserted: 2, duplicates: 0 })]);
        let summary = worker.drain_outbox_once(&sink).await.unwrap();

        assert_eq!(summary.attempted, 2);
        let remaining: Vec<_> = backend
            .get_external_outbox_transactions(10)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(remaining.is_empty());
        assert_eq!(sink.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn drain_handles_duplicate_key() {
        let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
        let metrics = Arc::new(ExternalDbMetrics::register());
        let config = ExternalDbConfig::new("mongodb://localhost:27017".to_string());
        let worker = ExternalDbWorker::new(config, backend.clone(), "MADARA_TEST".to_string(), metrics);

        let tx1 = make_validated_tx(3);
        let tx2 = make_validated_tx(4);
        backend.write_external_outbox(&tx1).unwrap();
        backend.write_external_outbox(&tx2).unwrap();

        let sink = FakeSink::new(vec![Ok(InsertManySummary { inserted: 1, duplicates: 1 })]);
        let summary = worker.drain_outbox_once(&sink).await.unwrap();

        assert_eq!(summary.attempted, 2);
        assert_eq!(summary.duplicates, 1);
        let remaining: Vec<_> = backend
            .get_external_outbox_transactions(10)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(remaining.is_empty());
    }

    #[tokio::test]
    async fn startup_sync_drains_existing_outbox() {
        let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
        let metrics = Arc::new(ExternalDbMetrics::register());
        let mut config = ExternalDbConfig::new("mongodb://localhost:27017".to_string());
        config.batch_size = 1;
        let worker = ExternalDbWorker::new(config, backend.clone(), "MADARA_TEST".to_string(), metrics);

        let tx1 = make_validated_tx(5);
        let tx2 = make_validated_tx(6);
        backend.write_external_outbox(&tx1).unwrap();
        backend.write_external_outbox(&tx2).unwrap();

        let sink = FakeSink::new(vec![
            Ok(InsertManySummary { inserted: 1, duplicates: 0 }),
            Ok(InsertManySummary { inserted: 1, duplicates: 0 }),
        ]);

        let drained = worker.startup_sync(&sink).await.unwrap();
        assert_eq!(drained, 2);
        assert_eq!(sink.calls.lock().unwrap().len(), 2);
    }

    struct FakeL1Source {
        batches: Mutex<Vec<Vec<L1Confirmation>>>,
    }

    impl FakeL1Source {
        fn new(batches: Vec<Vec<L1Confirmation>>) -> Self {
            Self { batches: Mutex::new(batches) }
        }
    }

    #[async_trait::async_trait]
    impl L1ConfirmationSource for FakeL1Source {
        async fn poll_confirmations(&mut self) -> anyhow::Result<Vec<L1Confirmation>> {
            Ok(self.batches.lock().unwrap().remove(0))
        }
    }

    #[tokio::test]
    async fn retention_deletes_after_delay() {
        let metrics = Arc::new(ExternalDbMetrics::register());
        let sink = FakeSink::new(Vec::new());
        let now = Instant::now();
        let confirmations = vec![L1Confirmation {
            block_number: 1,
            tx_hashes: vec![Felt::from_hex_unchecked("0x1"), Felt::from_hex_unchecked("0x2")],
        }];
        let mut scheduler = RetentionScheduler::new(FakeL1Source::new(vec![confirmations, Vec::new()]), Duration::from_secs(1), metrics);

        scheduler.tick(now, &sink).await.unwrap();
        assert!(sink.delete_calls.lock().unwrap().is_empty());

        scheduler.tick(now + Duration::from_secs(2), &sink).await.unwrap();
        assert_eq!(sink.delete_calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn retention_noop_without_confirmations() {
        let metrics = Arc::new(ExternalDbMetrics::register());
        let sink = FakeSink::new(Vec::new());
        let now = Instant::now();
        let mut scheduler = RetentionScheduler::new(FakeL1Source::new(vec![Vec::new()]), Duration::from_secs(1), metrics);

        scheduler.tick(now, &sink).await.unwrap();
        assert!(sink.delete_calls.lock().unwrap().is_empty());
    }

    #[test]
    fn retry_backoff_on_failure_caps() {
        let mut backoff = RetryBackoff::new(Duration::from_millis(100), Duration::from_millis(400));
        assert_eq!(backoff.next_delay(), Duration::from_millis(100));
        assert_eq!(backoff.next_delay(), Duration::from_millis(200));
        assert_eq!(backoff.next_delay(), Duration::from_millis(400));
        assert_eq!(backoff.next_delay(), Duration::from_millis(400));
        backoff.reset();
        assert_eq!(backoff.next_delay(), Duration::from_millis(100));
    }
}
