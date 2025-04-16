use async_trait::async_trait;
use mockall::automock;
use color_eyre::Result;

pub mod aws_sns;

#[automock]
#[async_trait]
pub trait Alerts: Send + Sync {
    /// To send an alert message to our alert service
    async fn send_alert_message(&self, message_body: String) -> Result<()>;
    async fn get_topic_name(&self) -> String;
    async fn create_alert(&self, topic_name: &str) -> Result<()>;
    async fn exists(&self) -> Result<bool>;
    async fn setup(&self) -> Result<()> {
        let topic_name = self.get_topic_name().await;
        if !self.exists().await? {
            self.create_alert(&topic_name).await?;
        } else {
            self.send_alert_message(format!("Topic {} already exists, skipping", topic_name)).await?;
        }
        Ok(())
    }
}
