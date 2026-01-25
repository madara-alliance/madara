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

pub mod config;
pub mod metrics;
pub mod mongodb;
pub mod service;
pub mod writer;

pub use config::ExternalDbConfig;
pub use service::ExternalDbService;
