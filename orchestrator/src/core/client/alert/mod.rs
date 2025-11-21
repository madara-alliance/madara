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

    /// Perform a health check on the alert service
    ///
    /// This method verifies that the alert service (e.g., AWS SNS) is accessible
    /// and the necessary permissions are in place.
    ///
    /// # Returns
    /// * `Ok(())` - If the alert service is healthy and accessible
    /// * `Err(AlertError)` - If the health check fails
    async fn health_check(&self) -> Result<(), AlertError>;
}
