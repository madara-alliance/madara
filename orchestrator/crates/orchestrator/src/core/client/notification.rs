use async_trait::async_trait;
use serde::Serialize;
use std::collections::HashMap;

use crate::error::Result;

/// Notification message
#[derive(Debug, Clone)]
pub struct Notification {
    /// Subject of the notification
    pub subject: String,

    /// Message body
    pub message: String,

    /// Message attributes
    pub attributes: HashMap<String, String>,
}

/// Trait defining notification service operations
#[async_trait]
pub trait NotificationClient: Send + Sync {
    /// Initialize the notification client
    async fn init(&self) -> Result<()>;

    /// Create a topic
    async fn create_topic(&self, name: &str) -> Result<String>;

    /// Delete a topic
    async fn delete_topic(&self, topic_arn: &str) -> Result<()>;

    /// List topics
    async fn list_topics(&self) -> Result<Vec<String>>;

    /// Subscribe to a topic
    async fn subscribe(&self, topic_arn: &str, protocol: &str, endpoint: &str) -> Result<String>;

    /// Unsubscribe from a topic
    async fn unsubscribe(&self, subscription_arn: &str) -> Result<()>;

    /// List subscriptions for a topic
    async fn list_subscriptions(&self, topic_arn: &str) -> Result<Vec<(String, String)>>;

    /// Publish a notification to a topic
    async fn publish(&self, topic_arn: &str, notification: &Notification) -> Result<String>;

    /// Publish a structured message to a topic
    async fn publish_structured<T>(&self, topic_arn: &str, subject: &str, message: &T) -> Result<String>
    where
        T: Serialize + Send + Sync;
}
