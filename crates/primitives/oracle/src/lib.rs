use async_trait::async_trait;

pub mod pragma;

#[async_trait]
pub trait Oracle: Send + Sync {
    async fn fetch_eth_strk_price(&self) -> anyhow::Result<(u128, u32)>;
    fn get_fetch_url(&self, base: String, quote: String) -> String;
    fn get_api_key(&self) -> &String;
}
