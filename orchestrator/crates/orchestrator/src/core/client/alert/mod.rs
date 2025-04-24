pub mod error;
pub(crate) mod sns;

use async_trait::async_trait;
use mockall::automock;

pub use error::AlertError;

/// AlertClient trait
#[automock]
#[async_trait]
pub trait AlertClient: Send + Sync {
    /// send_message sends a message to the alert client.
    ///
    /// # Arguments
    ///
    /// * `message_body` - The message body to send.
    ///
    /// # Returns
    async fn send_message(&self, message_body: String) -> Result<(), AlertError>;

    /// get_topic_name gets the topic name from the alert client.
    ///
    /// # Returns
    ///
    /// * `Result<String, AlertError>` - The topic name.
    async fn get_topic_name(&self) -> Result<String, AlertError>;
}
