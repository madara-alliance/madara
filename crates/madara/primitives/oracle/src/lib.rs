use async_trait::async_trait;

pub mod pragma;

#[async_trait]
pub trait Oracle: Send + Sync {
    async fn fetch_eth_strk_price(&self) -> anyhow::Result<(u128, u32)>;
}
