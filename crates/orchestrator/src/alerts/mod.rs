use async_trait::async_trait;
use mockall::automock;

pub mod aws_sns;

#[automock]
#[async_trait]
pub trait Alerts: Send + Sync {
    /// To send an alert message to our alert service
    async fn send_alert_message(&self, message_body: String) -> color_eyre::Result<()>;
    async fn get_topic_name(&self) -> String;
    async fn create_alert(&self, topic_name: &str) -> color_eyre::Result<()>;
    async fn setup(&self) -> color_eyre::Result<()> {
        let topic_name = self.get_topic_name().await;
        self.create_alert(&topic_name).await?;
        Ok(())
    }
}
