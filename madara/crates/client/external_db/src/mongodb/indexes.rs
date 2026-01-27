//! MongoDB index definitions.
//!
//! Index definitions for the mempool_transactions collection.
//! These indexes are created on service startup to optimize query patterns.
//!
//! Indexes:
//! - `{ "_id": 1 }` - Primary key (automatic)
//! - `{ "sender_address": 1, "arrived_at": -1 }` - For sender queries with time ordering
//! - `{ "arrived_at": -1 }` - For time-range queries
//! - `{ "status": 1, "arrived_at": -1 }` - For status queries (future use)
//! - `{ "tx_type": 1, "arrived_at": -1 }` - For transaction type queries
//! - `{ "sender_address": 1, "nonce": 1 }` - For sender + nonce queries (replay)
//! - `{ "block_number": 1 }` - For retention queries by block number

use mongodb::{bson::doc, IndexModel};

pub fn get_index_models() -> Vec<IndexModel> {
    vec![
        IndexModel::builder().keys(doc! { "sender_address": 1, "arrived_at": -1 }).build(),
        IndexModel::builder().keys(doc! { "arrived_at": -1 }).build(),
        IndexModel::builder().keys(doc! { "status": 1, "arrived_at": -1 }).build(),
        IndexModel::builder().keys(doc! { "tx_type": 1, "arrived_at": -1 }).build(),
        IndexModel::builder().keys(doc! { "sender_address": 1, "nonce": 1 }).build(),
        IndexModel::builder().keys(doc! { "block_number": 1 }).build(),
    ]
}
