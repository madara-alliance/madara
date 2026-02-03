//! External Database Integration for Madara Mempool
//!
//! This crate provides external database (MongoDB) support for Madara's mempool
//! using a durable outbox pattern. The goal is to ensure all transactions accepted
//! into the mempool are durably captured locally and asynchronously mirrored into
//! an external queryable database for archiving, backup, and replay.
//!
//! ## Architecture
//!
//! We use a transactional outbox pattern with RocksDB as the durable queue to avoid
//! dual-write loss while keeping MongoDB writes asynchronous. This preserves mempool
//! performance and guarantees capture of accepted transactions.
//!
//! ## Key Features
//!
//! - **Durability**: Accepted mempool txs are persisted even if external DB is down
//! - **Lifecycle Tracking**: Insert-only (no status updates)
//! - **Query Patterns**: Support queries by sender, time range, and type
//! - **Error Handling**: External DB writes are async and non-blocking
//! - **Retention**: Delete txs after their block is confirmed on L1 (configurable delay)
//!
//! ## Architecture (outbox pattern)
//!
//! We write accepted mempool txs to a local RocksDB outbox first, then drain
//! them asynchronously into MongoDB. This avoids dual-write loss while keeping
//! the mempool path fast.
//!
//! ```
//!      +---------------------------+
//!      | 1) tx accepted by mempool |
//!      +-------------+-------------+
//!                    |
//!                    v
//!      +---------------------------+
//!      | 2) write to outbox (RDB)  |
//!      +-------------+-------------+
//!                    |
//!                    v
//!      +---------------------------+
//!      | 3) async worker drains    |
//!      |    outbox -> MongoDB      |
//!      +-------------+-------------+
//!                    |
//!                    v
//!      +---------------------------+
//!      | 4) retention after L1     |
//!      |    confirmation + delay   |
//!      +---------------------------+
//! ```
//!
//! ## Failure behavior
//!
//! - **Mongo down / write failure**: outbox entries stay in RocksDB and are retried
//!   with exponential backoff. No node shutdown is triggered.
//! - **Strict outbox**: when enabled, mempool acceptance fails if outbox write fails.
//! - **Duplicates**: duplicate key errors in Mongo are treated as success so the
//!   outbox can be safely drained.
//!
//! ## Replay & ordering
//!
//! - WAL is an append log of accepted txs. Replays should be ordered by `arrived_at`
//!   (then a stable UUID tie-breaker).
//! - Some replays can still be rejected by the mempool (nonce rules, duplicates).
//! - Mempool DB persistence can conflict with WAL replay on restart. Prefer
//!   `--no-mempool-saving` for replay flows (see docs/mempool_wal_gotchas.md).

pub mod config;
pub mod metrics;
pub mod mongodb;
pub mod service;
pub mod writer;

pub use config::ExternalDbConfig;
pub use service::ExternalDbService;
