pub mod sns;

use async_trait::async_trait;
use mockall::automock;
use crate::OrchestratorResult;

#[automock]
#[async_trait]
pub trait AlertClient: Send + Sync {
    async fn send_alert_message(&self, message_body: String) -> OrchestratorResult<()>;
    async fn get_topic_name(&self) -> String;
    async fn create_alert(&self, topic_name: &str) -> OrchestratorResult<()>;
}
