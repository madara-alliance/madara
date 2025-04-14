pub mod error;
pub(crate) mod sns;

use async_trait::async_trait;
use mockall::automock;

pub use error::AlertError;

/// AlertClient trait
#[automock]
#[async_trait]
pub trait AlertClient: Send + Sync {
    async fn send_message(&self, message_body: String) -> Result<(), AlertError>;
    async fn get_topic_name(&self) -> Result<String, AlertError>;
}
