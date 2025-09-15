pub mod constant;
pub mod error;
pub mod mongodb;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use error::LockError;
#[cfg(feature = "with_mongodb")]
use mongodb::bson::serde_helpers::chrono_datetime_as_bson_datetime;
use serde::{Deserialize, Serialize};

/// Generic cache value that can store various data types
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum LockValue {
    String(String),
    Number(i64),
    Boolean(bool),
    Json(serde_json::Value),
}

impl From<String> for LockValue {
    fn from(s: String) -> Self {
        LockValue::String(s)
    }
}

impl From<&str> for LockValue {
    fn from(s: &str) -> Self {
        LockValue::String(s.to_string())
    }
}

impl From<i64> for LockValue {
    fn from(n: i64) -> Self {
        LockValue::Number(n)
    }
}

impl From<bool> for LockValue {
    fn from(b: bool) -> Self {
        LockValue::Boolean(b)
    }
}

impl From<serde_json::Value> for LockValue {
    fn from(v: serde_json::Value) -> Self {
        LockValue::Json(v)
    }
}

/// Lock information containing owner and acquisition time
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct LockInfo {
    pub _id: String, // Unique Identifier
    pub value: LockValue,
    #[cfg_attr(feature = "with_mongodb", serde(with = "chrono_datetime_as_bson_datetime"))]
    pub expires_at: DateTime<Utc>,
    #[cfg_attr(feature = "with_mongodb", serde(with = "chrono_datetime_as_bson_datetime"))]
    pub created_at: DateTime<Utc>,
    #[cfg_attr(feature = "with_mongodb", serde(with = "chrono_datetime_as_bson_datetime"))]
    pub updated_at: DateTime<Utc>,
    pub owner: Option<String>,
}

/// Result of lock acquisition attempts
#[derive(Debug, Clone, PartialEq)]
pub enum LockResult {
    Acquired,
    AlreadyHeld(String), // Contains the current owner
    Expired,
    Extended,
    Released,
    NotFound,
}

/// Redis-like cache service trait providing familiar APIs
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait LockClient: Send + Sync {
    /// Acquire a lock if it is available
    async fn acquire_lock_if_available(
        &self,
        key: &str,
        value: LockValue,
        expiry_seconds: u64,
        owner: Option<String>,
    ) -> Result<LockResult, LockError>;

    async fn acquire_lock(
        &self,
        key: &str,
        value: LockValue,
        expiry_seconds: u64,
        owner: Option<String>,
    ) -> Result<LockResult, LockError>;

    /// Release a lock if owned by the specified owner
    async fn release_lock(&self, key: &str, owner: Option<String>) -> Result<LockResult, LockError>;

    /// Extend an existing lock's expiry time if owned by the specified owner
    async fn extend_lock(
        &self,
        key: &str,
        value: LockValue,
        expiry_seconds: u64,
        owner: Option<String>,
    ) -> Result<LockResult, LockError>;

    /// Check if a lock exists and get its current owner
    async fn get_lock(&self, key: &str, owner: Option<String>) -> Result<LockInfo, LockError>;

    /// Check if a lock is currently held
    async fn is_locked(&self, key: &str) -> Result<bool, LockError>;

    /// Get time remaining before lock expires
    async fn get_lock_ttl(&self, key: &str) -> Result<Option<i64>, LockError>;
}
