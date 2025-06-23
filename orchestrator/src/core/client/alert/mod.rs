pub mod error;
pub(crate) mod sns;

use async_trait::async_trait;

pub use error::AlertError;

/// AlertClient trait
#[cfg_attr(test, mockall::automock)]
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
}
