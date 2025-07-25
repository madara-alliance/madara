use async_trait::async_trait;
use mp_convert::FixedPoint;

pub mod pragma;

#[async_trait]
pub trait Oracle: Send + Sync {
    async fn fetch_strk_per_eth(&self) -> anyhow::Result<FixedPoint>;
}
