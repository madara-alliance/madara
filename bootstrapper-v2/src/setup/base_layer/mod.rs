pub mod ethereum;
pub mod starknet;
use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait BaseLayerSetupTrait {
    /// This function does prerequisite setup for running the base layer setup.
    /// It should be called before the base layer setup.
    async fn init(&mut self) -> Result<()>;
    async fn setup(&mut self) -> Result<()>;
    fn post_madara_setup(&self, madara_addresses_path: &str) -> Result<()>;
}
