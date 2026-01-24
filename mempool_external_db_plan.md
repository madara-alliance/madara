# External MongoDB Transaction Store - Implementation Plan (Outbox-Based)

## Executive Summary

This document outlines the implementation plan for adding external database (MongoDB) support to Madara's mempool using a **durable outbox**. The goal is to ensure all transactions accepted into the mempool are durably captured locally and asynchronously mirrored into an external queryable database for archiving, backup, and replay.

### Key Requirements
- **Use Case**: Both historical archive AND mempool backup/replay source
- **Durability**: Accepted mempool txs must be persisted even if external DB is down
- **Lifecycle Tracking**: Insert-only (no status updates)
- **Query Patterns**: Support queries by sender, time range, and type
- **Error Handling**: External DB writes must be async and non-blocking
- **Retention**: Delete txs ~1 day after their block is submitted to L1 (configurable)

### Engineering Principles (Functional-Driven + TDD)
- **Short functions only**: prefer small, focused functions with single responsibility.
- **Strict TDD**: tests are written *before* implementation.
- **Meaningful tests**: no placeholder tests; cover happy paths and edge cases.
- **Balanced authority**: neither tests nor code should be "your boss" — both must reflect real behavior and intent.
- **End-to-end coverage**: include a full flow integration test (start Madara, submit txs, verify outbox → MongoDB → retention).

---

## Architecture Overview

We use a **transactional outbox** pattern with RocksDB as the durable queue to avoid dual-write loss while keeping Mongo writes asynchronous. This preserves mempool performance and guarantees capture of accepted transactions.

### Design Decision: Outbox-Only vs Channel
- **Outbox-only (chosen)**:
  - Pros: simpler, fewer failure modes, durable by default.
  - Cons: slightly higher write latency to Mongo (tick-based drain).
- **Outbox + channel (deferred)**:
  - Pros: lower latency to Mongo, higher throughput under load.
  - Cons: more complexity, harder crash reconciliation.

**Decision**: start with outbox-only for correctness and simplicity; consider adding channel later if needed.

### Design Decision: Outbox Location
- **Same RocksDB (chosen)**:
  - Pros: simpler operations, shared WAL/backup, fewer configs.
  - Cons: shared IO/compaction with core DB.
- **Separate RocksDB (deferred)**:
  - Pros: isolated IO/compaction, separate tuning.
  - Cons: more ops complexity, more failure modes.

**Decision**: store outbox in the existing RocksDB as a new column.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              Madara Node                                     │
│                                                                             │
│  ┌─────────────┐     ┌──────────────────┐     ┌──────────────────────────┐ │
│  │   RPC       │────▶│     Mempool      │────▶│  on_tx_added()           │ │
│  │  Endpoint   │     │                  │     │  ├─ RocksDB (sync)       │ │
│  └─────────────┘     │  InnerMempool    │     │  └─ Outbox (sync)         │ │
│                      │  ├─ ready_queue  │     └──────────┬───────────────┘ │
│                      │  ├─ by_tx_hash   │                │                 │
│                      │  └─ ...          │     └──────────┬───────────────┘ │
│                      └──────────────────┘                │                 │
│                                                          │                 │
│  ┌───────────────────────────────────────────────────────┼───────────────┐ │
│  │              ExternalDbService (background task)      │               │ │
│  │                                                       ▼               │ │
│  │  ┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐   │ │
│  │  │ Outbox Drainer  │───▶│  Batch Writer   │───▶│  MongoDB Client │   │ │
│  │  │ (RocksDB scan)  │    │  (configurable  │    │  (connection    │   │ │
│  │  │                 │    │   batch size)   │    │   pool)         │   │ │
│  │  └─────────────────┘    └─────────────────┘    └────────┬────────┘   │ │
│  └─────────────────────────────────────────────────────────┼────────────┘ │
└────────────────────────────────────────────────────────────┼──────────────┘
                                                             │
                                                             ▼
                                                    ┌────────────────┐
                                                    │    MongoDB     │
                                                    │  ┌──────────┐  │
                                                    │  │mempool_tx│  │
                                                    │  │collection│  │
                                                    │  └──────────┘  │
                                                    └────────────────┘
```

### Data Flow (Option A: Outbox-First, Strict Durability)

1. Transaction arrives via RPC endpoint
2. Transaction is validated
3. **Outbox write happens synchronously** (durable, strict)
4. Transaction is inserted into mempool
5. `on_tx_added()` is called for status tracking only (no outbox write here)
6. Background worker drains outbox, batches writes, and flushes to MongoDB periodically

---

## 1. New Crate Structure

```
crates/client/external-db/
├── Cargo.toml
├── src/
│   ├── lib.rs              # Public API, trait definitions
│   ├── config.rs           # Configuration structures
│   ├── writer.rs           # Outbox drainer + batch writer
│   ├── service.rs          # Service implementation
│   ├── mongodb/
│   │   ├── mod.rs          # MongoDB implementation
│   │   ├── client.rs       # Connection management
│   │   ├── models.rs       # MongoDB document models
│   │   └── indexes.rs      # Index creation
│   └── metrics.rs          # OpenTelemetry metrics
```

**Rationale**: Separate crate allows:
- Clean dependency management (mongodb driver only pulled when needed)
- Future extensibility to other databases (PostgreSQL, ClickHouse)
- Optional feature flag in node

**Additional local storage (mc-db)**:
- New RocksDB column for **external outbox**:
  - `mempool_external_outbox` (tx_hash → bincode(ValidatedTransaction))
  - Read/scan + delete APIs, similar to existing mempool persistence.

---

## 2. Configuration Design

### Configuration Structure

```rust
// crates/client/external-db/src/config.rs

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ExternalDbConfig {
    /// Enable external database storage
    pub enabled: bool,

    /// MongoDB connection URI
    /// Example: "mongodb://localhost:27017"
    pub mongodb_uri: String,

    /// Database name
    #[serde(default = "default_db_name")]
    pub database_name: String,  // Default: "madara_<chain_id>"

    /// Collection name for mempool transactions
    #[serde(default = "default_collection_name")]
    pub collection_name: String,  // Default: "mempool_transactions"

    /// Batch size for bulk writes
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,  // Default: 100

    /// Flush interval in milliseconds
    #[serde(default = "default_flush_interval_ms")]
    pub flush_interval_ms: u64,  // Default: 1000

    /// Connection pool size
    #[serde(default = "default_pool_size")]
    pub pool_size: u32,  // Default: 10

    /// Retention: delete txs N seconds after their block is submitted to L1
    #[serde(default = "default_retention_delay_secs")]
    pub retention_delay_secs: u64,  // Default: 86_400 (1 day)

    /// How often the retention sweeper checks for deletions
    #[serde(default = "default_retention_tick_secs")]
    pub retention_tick_secs: u64,  // Default: 300

    /// Required: failure to persist to outbox rejects mempool acceptance
    #[serde(default)]
    pub strict_outbox: bool,  // Default: true

    /// Retry backoff base (milliseconds)
    #[serde(default = "default_retry_backoff_ms")]
    pub retry_backoff_ms: u64,  // Default: 1000

    /// Retry backoff max (milliseconds)
    #[serde(default = "default_retry_backoff_max_ms")]
    pub retry_backoff_max_ms: u64,  // Default: 60000
}
```

**Database naming behavior**
- If `database_name` is not explicitly set, default to `madara_<chain_id>` in config builder.

### External Inputs & Integration Points (Required)
- **L1 confirmation events**: retention is strictly gated by L1 confirmation. The agent must wire a source that emits:
  - `block_number`
  - `tx_hashes` (all txs in that block)
  - `confirmed_at` (timestamp; optional if using "now")
- **Expected source candidates** (pick the real one in code):
  - L1 sync / settlement service
  - A "probe" or monitoring channel that already surfaces L1 submission status
- **If no source exists yet**:
  - Create a small internal interface (e.g., `L1ConfirmationSource`) and implement it in the service that already knows when a block is submitted to L1.
  - Do not add an age-only deletion fallback.

### CLI Flags

Add to a new module `madara/node/src/cli/external_db.rs` and flatten into `RunCmd`:

```
--external-db-enabled              Enable external database storage
--external-db-mongodb-uri <URI>    MongoDB connection URI
--external-db-database <NAME>      Database name (default: madara_<chain_id>)
--external-db-collection <NAME>    Collection name (default: mempool_transactions)
--external-db-batch-size <SIZE>    Batch size for bulk writes (default: 100)
--external-db-retention-secs <S>   Retention delay after L1 submission (default: 86400)
--external-db-retention-tick-secs <S> Retention sweeper interval (default: 300)
--external-db-strict-outbox        Reject mempool acceptance if outbox write fails
--external-db-retry-backoff-ms <MS> Base retry backoff (default: 1000)
--external-db-retry-backoff-max-ms <MS> Max retry backoff (default: 60000)
```

### Environment Variables

```
MADARA_EXTERNAL_DB_ENABLED=true
MADARA_EXTERNAL_DB_MONGODB_URI=mongodb://localhost:27017
MADARA_EXTERNAL_DB_DATABASE=madara_<chain_id>
MADARA_EXTERNAL_DB_COLLECTION=mempool_transactions
MADARA_EXTERNAL_DB_RETENTION_SECS=86400
MADARA_EXTERNAL_DB_RETENTION_TICK_SECS=300
MADARA_EXTERNAL_DB_STRICT_OUTBOX=true
MADARA_EXTERNAL_DB_RETRY_BACKOFF_MS=1000
MADARA_EXTERNAL_DB_RETRY_BACKOFF_MAX_MS=60000
```

---

## 3. MongoDB Document Schema

### Document Structure

```rust
// crates/client/external-db/src/mongodb/models.rs

use mongodb::bson::{doc, DateTime, Document};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MempoolTransactionDocument {
    /// Transaction hash (indexed, unique) - used as _id
    #[serde(rename = "_id")]
    pub tx_hash: String,

    /// Transaction type: "INVOKE", "DECLARE", "DEPLOY_ACCOUNT", "L1_HANDLER", "DEPLOY"
    pub tx_type: String,

    /// Transaction version: "V0", "V1", "V2", "V3"
    pub tx_version: String,

    /// Sender address (indexed)
    pub sender_address: String,

    /// Contract address (for deploy/L1Handler, same as sender for others)
    pub contract_address: String,

    /// Transaction nonce (as hex string)
    pub nonce: String,

    /// Arrival timestamp in mempool (indexed)
    pub arrived_at: DateTime,

    /// Insertion timestamp into MongoDB
    pub inserted_at: DateTime,

    /// Fee information (V0-V2)
    pub max_fee: Option<String>,

    /// Tip (V3 only)
    pub tip: Option<u64>,

    /// Resource bounds (V3 only)
    pub resource_bounds: Option<ResourceBoundsDoc>,

    /// Transaction signature
    pub signature: Vec<String>,

    /// Calldata (for Invoke transactions)
    pub calldata: Option<Vec<String>>,

    /// Class hash (for Declare/DeployAccount)
    pub class_hash: Option<String>,

    /// Compiled class hash (for Declare V2+)
    pub compiled_class_hash: Option<String>,

    /// Constructor calldata (for DeployAccount)
    pub constructor_calldata: Option<Vec<String>>,

    /// Contract address salt (for DeployAccount)
    pub contract_address_salt: Option<String>,

    /// Entry point selector (for L1Handler)
    pub entry_point_selector: Option<String>,

    /// Paid fee on L1 (for L1Handler only)
    pub paid_fee_on_l1: Option<String>,

    /// Full serialized ValidatedTransaction as bincode (for replay capability)
    pub raw_transaction: bson::Binary,

    /// Chain ID for multi-chain deployments
    pub chain_id: String,

    /// Status: "PENDING" (insert-only, can be extended later)
    pub status: String,

    /// Whether fee charging is enabled
    pub charge_fee: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceBoundsDoc {
    pub l1_gas_max_amount: u64,
    pub l1_gas_max_price: String,
    pub l2_gas_max_amount: u64,
    pub l2_gas_max_price: String,
    pub l1_data_gas_max_amount: Option<u64>,
    pub l1_data_gas_max_price: Option<String>,
}
```

### Indexes

Created on service startup:

```javascript
// Primary key (automatic)
{ "_id": 1 }

// Index for sender queries with time ordering
{ "sender_address": 1, "arrived_at": -1 }

// Index for time-range queries
{ "arrived_at": -1 }

// Index for status queries (future use)
{ "status": 1, "arrived_at": -1 }

// Index for transaction type queries
{ "tx_type": 1, "arrived_at": -1 }

// Compound index for sender + nonce queries (replay)
{ "sender_address": 1, "nonce": 1 }

// Retention is managed by service logic (delete after L1 submission + delay)
```

---

## 4. Outbox Drainer + Batch Writer Implementation

### Background Worker

```rust
// crates/client/external-db/src/writer.rs (continued)

pub struct ExternalDbWorker {
    config: ExternalDbConfig,
    backend: Arc<MadaraBackend>,
    client: Option<mongodb::Client>,
    collection: Option<mongodb::Collection<MempoolTransactionDocument>>,
    pending_batch: Vec<MempoolTransactionDocument>,
    metrics: Arc<ExternalDbMetrics>,
    chain_id: String,
}

impl ExternalDbWorker {
    pub async fn run(mut self, mut ctx: ServiceContext) -> anyhow::Result<()> {
        // Initialize MongoDB connection
        self.connect().await?;
        self.ensure_indexes().await?;
        self.startup_sync().await?;

        tracing::info!(
            "External DB worker started, connected to {}",
            self.config.mongodb_uri
        );

        let mut flush_interval = tokio::time::interval(
            Duration::from_millis(self.config.flush_interval_ms)
        );
        flush_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        // Retry strategy: exponential backoff with configurable base/max
        // self.config.retry_backoff_ms .. retry_backoff_max_ms

        loop {
            tokio::select! {
                // Graceful shutdown
                _ = ctx.cancelled() => {
                    tracing::info!("External DB worker shutting down, flushing remaining batch");
                    self.flush_batch().await?;
                    return Ok(());
                }

                // Periodic flush
                _ = flush_interval.tick() => {
                    if !self.pending_batch.is_empty() {
                        self.flush_batch().await?;
                    }
                    self.drain_outbox_once().await?;
                }
            }
        }
    }

    async fn connect(&mut self) -> anyhow::Result<()> {
        let mut client_options = mongodb::options::ClientOptions::parse(&self.config.mongodb_uri).await?;
        client_options.max_pool_size = Some(self.config.pool_size);
        client_options.min_pool_size = Some(1);
        client_options.connect_timeout = Some(Duration::from_secs(10));
        client_options.server_selection_timeout = Some(Duration::from_secs(10));

        let client = mongodb::Client::with_options(client_options)?;

        // Verify connection
        client.database("admin").run_command(doc! { "ping": 1 }).await?;

        let db = client.database(&self.config.database_name);
        let collection = db.collection(&self.config.collection_name);

        self.client = Some(client);
        self.collection = Some(collection);

        Ok(())
    }

    async fn ensure_indexes(&self) -> anyhow::Result<()> {
        let collection = self.collection.as_ref().expect("Not connected");

        let indexes = vec![
            IndexModel::builder()
                .keys(doc! { "sender_address": 1, "arrived_at": -1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "arrived_at": -1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "status": 1, "arrived_at": -1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "tx_type": 1, "arrived_at": -1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "sender_address": 1, "nonce": 1 })
                .build(),
        ];

        collection.create_indexes(indexes).await?;
        tracing::info!("External DB indexes ensured");

        Ok(())
    }

    async fn flush_batch(&mut self) -> anyhow::Result<()> {
        if self.pending_batch.is_empty() {
            return Ok(());
        }

        let batch = std::mem::take(&mut self.pending_batch);
        let count = batch.len();
        let start = std::time::Instant::now();

        let collection = self.collection.as_ref().expect("Not connected");

        // Use insert_many with ordered: false for best performance
        // Duplicates will fail individually but won't stop the batch
        match collection.insert_many(&batch).ordered(false).await {
            Ok(result) => {
                let duration = start.elapsed();
                tracing::debug!(
                    "Wrote {} transactions to external DB in {:?}",
                    result.inserted_ids.len(),
                    duration
                );
                self.metrics.transactions_written.add(result.inserted_ids.len() as u64, &[]);
                self.metrics.write_latency.record(duration.as_secs_f64(), &[]);
            }
            Err(e) => {
                // Check if it's a bulk write error (some succeeded, some failed)
                if let Some(bulk_err) = e.get_ref().and_then(|e| e.downcast_ref::<mongodb::error::BulkWriteError>()) {
                    let inserted = count - bulk_err.write_errors.len();
                    tracing::debug!(
                        "Partial write: {} inserted, {} duplicates/errors",
                        inserted,
                        bulk_err.write_errors.len()
                    );
                    self.metrics.transactions_written.add(inserted as u64, &[]);
                } else {
                    tracing::error!("Failed to write batch to external DB: {e:#}");
                    self.metrics.connection_errors.add(1, &[]);

                    // Put batch back for retry on next flush
                    self.pending_batch = batch;
                }
            }
        }

        Ok(())
    }

    fn convert_to_document(&self, tx: ValidatedTransaction) -> anyhow::Result<MempoolTransactionDocument> {
        // Implementation converts ValidatedTransaction to MempoolTransactionDocument
        // Extracts all fields, serializes raw transaction for replay
        // ...
    }
}
```

### Outbox Behavior (RocksDB-backed)

- **Write path (sync, durable)**:
  - In `add_tx` (before mempool insertion), write to outbox column:
    - `tx_hash -> bincode(ValidatedTransaction)`
  - If write fails: **reject mempool acceptance** (strict outbox required)
  - If mempool insertion fails after outbox write: delete outbox entry (rollback)
- **Drain path (async)**:
  - Background worker periodically scans outbox entries (bounded batch)
  - For each entry, try to insert into MongoDB
  - On success or duplicate key, delete from outbox

**Ownership**
- Outbox is part of `MadaraBackend` (db APIs alongside existing mempool persistence).

### Outbox API (Expected Interface)
```rust
// mc-db/src/rocksdb/external_outbox.rs
impl RocksDBStorageInner {
    pub fn write_external_outbox(&self, tx: &ValidatedTransaction) -> Result<()>;
    pub fn delete_external_outbox(&self, tx_hash: Felt) -> Result<()>;
    pub fn iter_external_outbox(&self) -> impl Iterator<Item = Result<ValidatedTransaction>>;
    pub fn external_outbox_count(&self) -> Result<usize>;
}

// Exposed via MadaraBackend wrapper
impl<D: MadaraStorageRead> MadaraBackend<D> {
    pub fn get_external_outbox_transactions(&self) -> impl Iterator<Item = Result<ValidatedTransaction>>;
}
impl<D: MadaraStorageWrite> MadaraBackend<D> {
    pub fn write_external_outbox(&self, tx: &ValidatedTransaction) -> Result<()>;
    pub fn delete_external_outbox(&self, tx_hash: Felt) -> Result<()>;
}
```

---

## 5. Service Implementation

```rust
// crates/client/external-db/src/service.rs

use mp_utils::service::{MadaraServiceId, PowerOfTwo, Service, ServiceContext, ServiceId, ServiceRunner};

pub struct ExternalDbService {
    worker: Option<ExternalDbWorker>,
    config: ExternalDbConfig,
}

impl ExternalDbService {
    pub fn new(
        config: ExternalDbConfig,
        chain_id: String,
        backend: Arc<MadaraBackend>,
    ) -> anyhow::Result<Self> {
        let metrics = Arc::new(ExternalDbMetrics::register());

        let worker = ExternalDbWorker {
            config: config.clone(),
            backend,
            client: None,
            collection: None,
            pending_batch: Vec::with_capacity(config.batch_size),
            metrics,
            chain_id,
        };

        Ok(Self {
            worker: Some(worker),
            config,
        })
    }
}

#[async_trait::async_trait]
impl Service for ExternalDbService {
    async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        let worker = self.worker.take().expect("Service already started");

        runner.service_loop(move |ctx| async move {
            worker.run(ctx).await
        });

        Ok(())
    }
}

impl ServiceId for ExternalDbService {
    fn svc_id(&self) -> PowerOfTwo {
        MadaraServiceId::ExternalDb.svc_id()
    }
}
```

**Service ID update**
- Add `ExternalDb` to `MadaraServiceId` in `madara/crates/primitives/utils/src/service.rs`.

---

## 6. Mempool Integration

### Changes to `crates/client/mempool/src/lib.rs`

```rust
// Add new field to Mempool struct
pub struct Mempool<D: MadaraStorageRead = RocksDBStorage> {
    backend: Arc<MadaraBackend<D>>,
    inner: MempoolInnerWithNotify,
    metrics: MempoolMetrics,
    config: MempoolConfig,
    ttl: Option<Duration>,
    watch_transaction_status: TopicWatchPubsub<Felt, Option<TransactionStatus>>,
    preconfirmed_transactions_statuses: DashMap<Felt, PreConfirmationStatus>,

    // NEW: External DB enabled flag (controls outbox writes)
    external_db_enabled: bool,
}

// Update constructor
impl<D: MadaraStorageRead> Mempool<D> {
    pub fn new(
        backend: Arc<MadaraBackend<D>>,
        config: MempoolConfig,
        external_db_enabled: bool,  // NEW parameter
    ) -> Self {
        Mempool {
            inner: MempoolInnerWithNotify::new(backend.chain_config()),
            ttl: backend.chain_config().mempool_ttl,
            backend,
            config,
            metrics: MempoolMetrics::register(),
            watch_transaction_status: Default::default(),
            preconfirmed_transactions_statuses: Default::default(),
            external_db_enabled,  // NEW
        }
    }
}

// Update add_tx flow (outbox-first, strict durability)
impl<D: MadaraStorageRead + MadaraStorageWrite> Mempool<D> {
    async fn add_tx(&self, tx: ValidatedTransaction, is_new_tx: bool) -> Result<(), MempoolInsertionError> {
        tracing::debug!("Accepting transaction tx_hash={:#x} is_new_tx={is_new_tx}", tx.hash);

        if is_new_tx && self.external_db_enabled {
            // NEW: External outbox (sync, durable) before mempool insertion
            self.backend
                .write_external_outbox(&tx)
                .map_err(|e| MempoolInsertionError::Internal(anyhow::anyhow!("outbox write failed: {e:#}")))?;
        }

        let now = TxTimestamp::now();
        let account_nonce =
            self.backend.view_on_latest().get_contract_nonce(&tx.contract_address)?.unwrap_or(Felt::ZERO);
        let mut removed_txs = smallvec::SmallVec::<[ValidatedTransaction; 1]>::new();

        let (ret, summary) = {
            let mut lock = self.inner.write().await;
            let ret = lock.insert_tx(now, tx.clone(), Nonce(account_nonce), &mut removed_txs);
            (ret, lock.summary())
        };

        // Roll back outbox on mempool insert failure
        if ret.is_err() && self.external_db_enabled {
            let _ = self.backend.delete_external_outbox(tx.hash);
        }

        self.metrics.record_mempool_state(&summary);
        self.on_txs_removed(&removed_txs);

        if ret.is_ok() {
            self.on_tx_added(&tx, is_new_tx);
        }

        ret.map_err(Into::into)
    }

    fn on_tx_added(&self, tx: &ValidatedTransaction, is_new_tx: bool) {
        tracing::debug!("Accepted transaction tx_hash={:#x}", tx.hash);

        if is_new_tx {
            self.metrics.accepted_transaction_counter.add(1, &[]);

            // Existing: RocksDB persistence (synchronous)
            if self.config.save_to_db {
                if let Err(err) = self.backend.write_saved_mempool_transaction(tx) {
                    tracing::error!("Could not add mempool transaction to database: {err:#}");
                }
            }
        }

        // Existing status tracking code...
        if let dashmap::Entry::Vacant(entry) = self.preconfirmed_transactions_statuses.entry(tx.hash) {
            let status = PreConfirmationStatus::Received(Arc::new(tx.clone()));
            entry.insert(status.clone());
            self.watch_transaction_status.publish(&tx.hash, Some(TransactionStatus::Preconfirmed(status)));
        }
    }
}
```

**Strict outbox behavior**
- `strict_outbox = true` is required; outbox write failure rejects mempool acceptance.

**Independence from `save_to_db`**
- External DB + outbox are controlled by `ExternalDbConfig` and do not depend on `MempoolConfig::save_to_db`.

---

## 7. Node Integration

### Changes to `node/src/service/mempool.rs`

```rust
pub struct MempoolService {
    mempool: Arc<Mempool>,
}

impl MempoolService {
    pub fn new(
        run_cmd: &RunCmd,
        backend: Arc<MadaraBackend>,
        external_db_enabled: bool,  // NEW
    ) -> Self {
        Self {
            mempool: Arc::new(Mempool::new(
                Arc::clone(&backend),
                MempoolConfig { save_to_db: !run_cmd.validator_params.no_mempool_saving },
                external_db_enabled,  // NEW
            )),
        }
    }
}
```

### Changes to `node/src/main.rs`

```rust
// In main() or run() function:

// Load external DB config from CLI/config
let external_db_config = run_cmd.external_db_config()?;

// Create external DB service if enabled
let external_db_enabled = external_db_config.enabled;
let external_db_service = if external_db_config.enabled {
    let chain_id = backend.chain_config().chain_id.to_string();
    let service = ExternalDbService::new(external_db_config, chain_id, backend.clone())?;
    Some(service)
} else {
    None
};

// Pass external DB enabled flag to mempool service
let service_mempool = MempoolService::new(&run_cmd, backend.clone(), external_db_enabled);

// Build service monitor
let mut app = ServiceMonitor::default()
    .with(service_mempool)?
    .with(service_l1_sync)?
    // ... other services
    ;

// Register external DB service if enabled
if let Some(service) = external_db_service {
    app = app.with(service)?;
    app.activate(MadaraServiceId::ExternalDb);
}
```

### Changes to `madara/node/src/cli/external_db.rs`

```rust
#[derive(Debug, Clone, Parser)]
pub struct ExternalDbParams {
    /// Enable external database storage for mempool transactions
    #[arg(long, env = "MADARA_EXTERNAL_DB_ENABLED", default_value = "false")]
    pub external_db_enabled: bool,

    /// MongoDB connection URI
    #[arg(long, env = "MADARA_EXTERNAL_DB_MONGODB_URI")]
    pub external_db_mongodb_uri: Option<String>,

    /// Database name
    #[arg(long, env = "MADARA_EXTERNAL_DB_DATABASE", default_value = "madara_<chain_id>")]
    pub external_db_database: String,

    /// Collection name for mempool transactions
    #[arg(long, env = "MADARA_EXTERNAL_DB_COLLECTION", default_value = "mempool_transactions")]
    pub external_db_collection: String,

    /// Batch size for bulk writes
    #[arg(long, env = "MADARA_EXTERNAL_DB_BATCH_SIZE", default_value = "100")]
    pub external_db_batch_size: usize,

    /// Flush interval in milliseconds
    #[arg(long, env = "MADARA_EXTERNAL_DB_FLUSH_INTERVAL_MS", default_value = "1000")]
    pub external_db_flush_interval_ms: u64,

    /// Retention delay after L1 submission (seconds)
    #[arg(long, env = "MADARA_EXTERNAL_DB_RETENTION_SECS", default_value = "86400")]
    pub external_db_retention_secs: u64,

    /// Retention sweeper interval (seconds)
    #[arg(long, env = "MADARA_EXTERNAL_DB_RETENTION_TICK_SECS", default_value = "300")]
    pub external_db_retention_tick_secs: u64,


    /// Required: reject mempool acceptance if outbox write fails
    #[arg(long, env = "MADARA_EXTERNAL_DB_STRICT_OUTBOX", default_value = "true")]
    pub external_db_strict_outbox: bool,

    /// Base retry backoff (ms)
    #[arg(long, env = "MADARA_EXTERNAL_DB_RETRY_BACKOFF_MS", default_value = "1000")]
    pub external_db_retry_backoff_ms: u64,

    /// Max retry backoff (ms)
    #[arg(long, env = "MADARA_EXTERNAL_DB_RETRY_BACKOFF_MAX_MS", default_value = "60000")]
    pub external_db_retry_backoff_max_ms: u64,
}
```

---

## 8. Metrics & Observability

```rust
// crates/client/external-db/src/metrics.rs

use opentelemetry::metrics::{Counter, Gauge, Histogram, Meter};

pub struct ExternalDbMetrics {
    /// Total transactions successfully written to external DB
    pub transactions_written: Counter<u64>,

    /// Transactions deleted by retention
    pub transactions_deleted: Counter<u64>,

    /// Batch write latency histogram
    pub write_latency: Histogram<f64>,

    /// MongoDB connection/write errors
    pub mongodb_write_errors: Counter<u64>,

    /// Current batch size
    pub batch_size: Gauge<u64>,

    /// Outbox write errors
    pub outbox_write_errors: Counter<u64>,

    /// Current outbox size
    pub outbox_size: Gauge<u64>,

    /// Retry attempts due to MongoDB failures
    pub retry_count: Counter<u64>,
}

impl ExternalDbMetrics {
    pub fn register() -> Self {
        let meter = opentelemetry::global::meter("madara.external_db");

        Self {
            transactions_written: meter
                .u64_counter("external_db.transactions_written")
                .with_description("Total transactions written to external DB")
                .build(),
            transactions_deleted: meter
                .u64_counter("external_db.transactions_deleted")
                .with_description("Transactions deleted by retention")
                .build(),
            write_latency: meter
                .f64_histogram("external_db.write_latency_seconds")
                .with_description("Batch write latency in seconds")
                .build(),
            mongodb_write_errors: meter
                .u64_counter("external_db.mongodb_write_errors")
                .with_description("MongoDB connection/write errors")
                .build(),
            batch_size: meter
                .u64_gauge("external_db.batch_size")
                .with_description("Current batch size")
                .build(),
            outbox_write_errors: meter
                .u64_counter("external_db.outbox_write_errors")
                .with_description("Outbox write errors")
                .build(),
            outbox_size: meter
                .u64_gauge("external_db.outbox_size")
                .with_description("Current outbox size")
                .build(),
            retry_count: meter
                .u64_counter("external_db.retry_count")
                .with_description("Retry attempts due to MongoDB failures")
                .build(),
        }
    }
}
```

---

## 9. Recovery, Outbox Drain & Consistency

### Startup Sync (Required)

On startup, the outbox drainer scans all outbox entries and attempts to insert them into MongoDB. This guarantees replay after crashes or Mongo downtime:

```rust
impl ExternalDbWorker {
    /// Sync transactions from outbox after crash / downtime
    async fn startup_sync(&mut self) -> anyhow::Result<usize> {
        let mut synced = 0;

        for tx_result in self.backend.get_external_outbox_transactions() {
            let tx = tx_result?;
            self.pending_batch.push(self.convert_to_document(tx)?);
            synced += 1;

            if self.pending_batch.len() >= self.config.batch_size {
                self.flush_batch().await?;
            }
        }

        if synced > 0 {
            tracing::info!("Synced {} transactions from RocksDB to external DB", synced);
        }

        Ok(synced)
    }

    // Duplicates handled by insert_many ordered:false + delete from outbox on duplicate.
}
```

### Retention Sweeper (L1 Submission + Delay)
- The service accepts **L1 submission events** (block + tx hashes) from a probe or existing L1 sync/settlement component.
- A retention task schedules deletion of those tx hashes after `retention_delay_secs`.
- **No age-based fallback**: deletion is strictly gated by L1 confirmation.

**Implementation note**: keep this dynamic and low-overhead; schedule only small delete batches per tick.

---

## 10. Milestones (Buildable + TDD-Gated)

Each milestone must build (`cargo build`) and the tests for that milestone must pass. Tests are written first and must cover happy + edge cases. Functions must remain short and focused.

### Milestone 0 — Scaffolding & Config (Buildable)
**Tests (first):**
- None required yet beyond compile checks.

**Work:**
- Create `mc-external-db` crate skeleton.
- Add `ExternalDbParams` + figment wiring.
- Add `MadaraServiceId::ExternalDb`.

---

### Milestone 1 — Outbox Persistence (Buildable, TDD)
**Tests (first):**
- `outbox_write_read_roundtrip`
- `outbox_delete_removes_entry`
- `outbox_iter_bounded_batch`
- Edge: duplicate writes for same `tx_hash` replace/update behavior.

**Work:**
- Add outbox column + `mc-db` APIs.
- Ensure RocksDB integration + metrics hooks.

---

### Milestone 2 — Mempool Integration (Buildable, TDD)
**Tests (first):**
- `mempool_accept_writes_outbox`
- `mempool_reject_rolls_back_outbox` (delete on insertion failure)
- Edge: `strict_outbox = true` rejects mempool acceptance on outbox failure.

**Work:**
- Wire outbox write into `add_tx` **before** mempool insertion.
- Roll back outbox on mempool insertion failure.
- Enforce strict outbox behavior (reject on outbox write failure).

---

### Milestone 3 — Outbox Drainer + Mongo Writer (Buildable, TDD)
**Tests (first):**
- `drain_outbox_inserts_to_mongo`
- `drain_handles_duplicate_key`
- `drain_retry_backoff_on_failure`
- Edge: backoff max cap; ensures no tight loop.

**Work:**
- Implement worker drain loop with configurable exponential backoff.
- Insert with `ordered: false`.
- Delete outbox entry on success or duplicate.

---

### Milestone 4 — Startup Sync + Retention (Buildable, TDD)
**Tests (first):**
- `startup_sync_drains_existing_outbox`
- `retention_deletes_after_delay`
- Edge: retention no-op when L1 submission data missing and fallback disabled.

**Work:**
- Add startup drain invocation.
- Add retention scheduler with L1 submission events + optional fallback.

---

### Milestone 5 — End-to-End Integration Test (Buildable)
**Tests (first):**
- Start Madara, submit tx(s), assert:
  - Outbox persisted locally
  - MongoDB receives tx
  - Retention delete occurs after delay (or mock time)

**Work:**
- Testcontainers (MongoDB) + test harness for Madara startup and tx submission.

---

## 11. File Changes Summary

| File | Change Type | Description |
|------|-------------|-------------|
| `crates/client/external-db/Cargo.toml` | **New** | New crate manifest |
| `crates/client/external-db/src/lib.rs` | **New** | Public API exports |
| `crates/client/external-db/src/config.rs` | **New** | Configuration structures |
| `crates/client/external-db/src/writer.rs` | **New** | Outbox drainer + batch writer |
| `crates/client/external-db/src/service.rs` | **New** | Service trait implementation |
| `crates/client/external-db/src/metrics.rs` | **New** | OpenTelemetry metrics |
| `crates/client/external-db/src/mongodb/mod.rs` | **New** | MongoDB module |
| `crates/client/external-db/src/mongodb/models.rs` | **New** | Document models |
| `crates/client/external-db/src/mongodb/client.rs` | **New** | Connection management |
| `crates/client/external-db/src/mongodb/indexes.rs` | **New** | Index definitions |
| `crates/client/db/src/rocksdb/external_outbox.rs` | **New** | External outbox column access |
| `crates/client/db/src/rocksdb/mod.rs` | **Modify** | Wire outbox column |
| `crates/client/mempool/src/lib.rs` | **Modify** | Add external_db_outbox |
| `node/src/main.rs` | **Modify** | Wire up ExternalDbService + outbox |
| `node/src/cli/external_db.rs` | **New** | External DB CLI flags |
| `node/src/cli/mod.rs` | **Modify** | Flatten ExternalDbParams into RunCmd |
| `node/src/service/mempool.rs` | **Modify** | Accept external_db_outbox |
| `crates/primitives/utils/src/service.rs` | **Modify** | Add MadaraServiceId::ExternalDb |
| `Cargo.toml` (workspace) | **Modify** | Add new crate to workspace |

---

## 12. Dependencies

```toml
# crates/client/external-db/Cargo.toml

[package]
name = "mc-external-db"
version = "0.1.0"
edition = "2021"
description = "External database integration for Madara mempool"

[dependencies]
# MongoDB driver
mongodb = { version = "3.1", features = ["tokio-runtime"] }
bson = "2"

# Async runtime
tokio = { version = "1", features = ["sync", "time"] }

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
bincode = "1"

# Error handling
thiserror = "1"
anyhow = "1"

# Logging
tracing = "0.1"

# Metrics
opentelemetry = "0.28"

# Internal crates
mp-transactions = { path = "../../primitives/transactions" }
mp-convert = { path = "../../primitives/convert" }
mp-utils = { path = "../../primitives/utils" }
mc-db = { path = "../db" }

[dev-dependencies]
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
testcontainers = "0.15"
testcontainers-modules = { version = "0.3", features = ["mongo"] }
```

---

## 13. Testing Strategy

**Policy**
- All tests are written before implementation (strict TDD).
- Tests must cover happy paths and edge cases and represent real behavior.
- End-to-end test is mandatory to validate full flow.

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_batch_flush_on_size() {
        // Test that batch flushes when reaching batch_size
    }

    #[tokio::test]
    async fn test_batch_flush_on_interval() {
        // Test that batch flushes on timer even if not full
    }

    #[tokio::test]
    async fn test_outbox_durable_write() {
        // Write to outbox, restart worker, verify drain to MongoDB
    }

    #[tokio::test]
    async fn test_raw_transaction_replay_roundtrip() {
        // Serialize ValidatedTransaction -> raw -> deserialize -> validate equality
    }
}
```

### Test Pseudocode (Representative, Non-Verbose)

**Outbox write before mempool insert**
```
given mempool with external_db_outbox enabled
when add_tx(tx)
then outbox.write(tx) happens before inner.insert_tx
and if outbox.write fails -> add_tx returns error
```

**Rollback on mempool insert failure**
```
given outbox.write succeeds
and inner.insert_tx returns Err
when add_tx(tx)
then outbox.delete(tx.hash) is called
and tx is not in mempool
```

**Drain inserts into Mongo and deletes outbox**
```
given outbox contains txA, txB
and mongo insert_many succeeds
when drain_outbox_once()
then mongo has txA, txB
and outbox entries are deleted
```

**Duplicate handling**
```
given outbox contains txA
and mongo already has txA (_id duplicate)
when drain_outbox_once()
then mongo insert reports duplicate for txA
and outbox entry for txA is deleted
```

**Retry backoff**
```
given mongo insert fails with connection error
when drain_outbox_once()
then retry_delay increases exponentially up to max
and retry_count metric increments
```

**Retention via L1 confirmation**
```
given L1 confirmation event for block N with tx hashes [h1,h2]
when retention_delay elapses
then delete_many({_id in [h1,h2]}) is issued
and transactions_deleted metric increments
```

**No deletion without L1 confirmation**
```
given tx is old but no L1 confirmation event received
when retention sweeper runs
then no delete is issued
```

**End-to-end flow**
```
start madara + mongo
submit tx
assert outbox persisted locally
wait for drain tick
assert mongo has tx
emit L1 confirmation for containing block
wait retention_delay
assert tx deleted in mongo
```

### Integration Tests

```rust
#[cfg(test)]
mod integration_tests {
    use testcontainers::{clients::Cli, images::mongo::Mongo};

    #[tokio::test]
    async fn test_mongodb_write_and_read() {
        let docker = Cli::default();
        let mongo_container = docker.run(Mongo::default());
        let port = mongo_container.get_host_port_ipv4(27017);

        let config = ExternalDbConfig {
            enabled: true,
            mongodb_uri: format!("mongodb://localhost:{}", port),
            ..Default::default()
        };

        // Start service, write transactions, verify in MongoDB
    }

    #[tokio::test]
    async fn test_graceful_shutdown_flushes_pending() {
        // Start service, write transactions, shutdown, verify all flushed
    }

    #[tokio::test]
    async fn test_duplicate_handling() {
        // Write same transaction twice, verify no error and single entry
    }

    #[tokio::test]
    async fn test_retention_deletes_after_delay() {
        // Simulate L1 submission event, wait delay, verify deletion
    }
}
```

### Stress Tests

```rust
#[tokio::test]
#[ignore] // Run manually
async fn stress_test_high_volume() {
    // Write 100k transactions, measure throughput and latency
    // Verify no drops under normal conditions
    // Measure memory usage
}
```

---

## 14. Usage Examples

### Running with External DB

```bash
# Using CLI flags
cargo run --release -- \
    --network mainnet \
    --external-db-enabled \
    --external-db-mongodb-uri "mongodb://localhost:27017" \
    --external-db-database "madara_mainnet" \
    --external-db-retention-secs 86400 \
    --external-db-batch-size 200

# Using environment variables
export MADARA_EXTERNAL_DB_ENABLED=true
export MADARA_EXTERNAL_DB_MONGODB_URI="mongodb://user:pass@cluster.mongodb.net"
export MADARA_EXTERNAL_DB_DATABASE="madara_prod"
export MADARA_EXTERNAL_DB_RETENTION_SECS=86400
cargo run --release -- --network mainnet
```

### Querying MongoDB

```javascript
// Find all transactions from a sender
db.mempool_transactions.find({
    sender_address: "0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7"
}).sort({ arrived_at: -1 })

// Find transactions in time range
db.mempool_transactions.find({
    arrived_at: {
        $gte: ISODate("2024-01-01T00:00:00Z"),
        $lt: ISODate("2024-01-02T00:00:00Z")
    }
})

// Find all invoke transactions
db.mempool_transactions.find({
    tx_type: "INVOKE"
}).limit(100)

// Get transaction count by type
db.mempool_transactions.aggregate([
    { $group: { _id: "$tx_type", count: { $sum: 1 } } }
])

// Find transactions by sender and nonce (for replay)
db.mempool_transactions.find({
    sender_address: "0x...",
    nonce: "0x5"
})
```

---

## 15. Future Extensions

This design supports:

1. **Other Databases**: Add PostgreSQL, ClickHouse implementations behind the same trait
2. **Lifecycle Tracking**: Add status updates if needed later (extend document + retention rules)
3. **Event Streaming**: Could emit to Kafka/NATS instead of/in addition to MongoDB
4. **Sharding**: MongoDB sharding key could be `sender_address` or time-based
5. **Read API**: Add query methods for reading back from external DB
6. **Replay Service**: Use stored transactions for replay/recovery scenarios
7. **External Queue**: Swap outbox drainer to push to Kafka/NATS for cross-system mirroring

---

## 16. Rollout Plan

### Phase 1: Core Implementation
1. Create `mc-external-db` crate with MongoDB support
2. Add RocksDB outbox column + API in `mc-db`
3. Implement outbox drain + async writer with batching
4. Add configuration and CLI flags
5. Integrate with mempool

### Phase 2: Testing & Validation
1. Unit tests for all components
2. Integration tests with testcontainers
3. Manual testing on devnet
4. Performance benchmarking

### Phase 3: Documentation & Release
1. Update CLAUDE.md with new service
2. Add operator documentation
3. Add configuration examples
4. Release as optional feature

---

## Appendix: Current Codebase Reference

### Key Files

| File | Purpose |
|------|---------|
| `crates/client/mempool/src/lib.rs` | Main mempool implementation, `on_tx_added()` hook |
| `crates/client/db/src/rocksdb/mempool.rs` | Current RocksDB mempool persistence |
| `crates/client/db/src/rocksdb/external_outbox.rs` | External DB outbox persistence |
| `crates/primitives/transactions/src/validated.rs` | `ValidatedTransaction` struct |
| `crates/primitives/transactions/src/lib.rs` | Transaction type definitions |
| `node/src/main.rs` | Service composition |
| `node/src/service/mempool.rs` | Mempool service wrapper |
| `crates/primitives/utils/src/service.rs` | Service trait definitions |

### ValidatedTransaction Fields

```rust
pub struct ValidatedTransaction {
    pub transaction: Transaction,      // Core transaction data
    pub paid_fee_on_l1: Option<u128>,  // L1Handler only
    pub contract_address: Felt,         // Sender or deployed address
    pub arrived_at: TxTimestamp,        // Arrival time (millis)
    pub declared_class: Option<ConvertedClass>,  // Declare only
    pub hash: Felt,                     // Transaction hash
    pub charge_fee: bool,               // Fee execution flag
}
```

### Transaction Types

- `Invoke` (V0, V1, V3)
- `Declare` (V0, V1, V2, V3)
- `DeployAccount` (V1, V3)
- `Deploy` (V0, legacy)
- `L1Handler` (V0)

---

## Open Questions & Decisions (Resolved)
- **Retention delete logic**: strictly requires L1 confirmation; no age-based fallback.
- **Outbox on mempool insertion failure**: delete/rollback outbox entry (only accepted txs are archived).
- **Outbox ownership**: part of `MadaraBackend` (db APIs like existing mempool persistence).
- **Channel vs outbox-only**: outbox-only drain (no in-memory channel).
- **Outbox write timing**: outbox write happens before mempool insertion; rollback on insertion failure.
- **Retry strategy**: configurable exponential backoff with sensible defaults (max cap).
- **Metrics**: include outbox size, outbox write errors, Mongo write errors/latency, retry count.
- **Capture L1→L2 handler txs?** Yes, they go through `ValidatedTransaction` and are captured during `add_tx` (outbox-first).
- **Strict durability?** Use synchronous outbox write; Mongo writes are async.
- **Accepted-only?** Only transactions accepted into mempool are captured.
- **Replay fidelity**: Store raw `ValidatedTransaction` bincode and verify via roundtrip tests.
- **Database naming**: if not provided, default to `madara_<chain_id>`.
