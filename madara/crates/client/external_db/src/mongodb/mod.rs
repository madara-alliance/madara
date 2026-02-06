//! MongoDB implementation for external database.

pub mod client;
pub mod indexes;
pub mod models;

pub use client::MongoClient;
pub use models::{MempoolTransactionDocument, ResourceBoundsDoc};
