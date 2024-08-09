use dp_block::header::{GasPrices, L1DataAvailabilityMode};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct GasPriceProvider {
    gas_prices: Arc<Mutex<GasPrices>>,
}

impl GasPriceProvider {
    pub fn new() -> Self {
        GasPriceProvider { gas_prices: Arc::new(Mutex::new(GasPrices::default())) }
    }
}

impl Default for GasPriceProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// This trait enables the block production task to fill in the L1 info.
/// Gas prices and DA mode
#[async_trait::async_trait]
pub trait L1DataProvider: Send + Sync {
    /// Get L1 data gas prices. This needs an oracle for STRK prices.
    async fn get_gas_prices(&self) -> GasPrices;
    async fn set_gas_prices(&self, new_prices: GasPrices);
    async fn update_eth_l1_gas_price(&self, new_price: u128);
    async fn update_eth_l1_data_gas_price(&self, new_price: u128);
    async fn update_strk_l1_gas_price(&self, new_price: u128);
    async fn update_strk_l1_data_gas_price(&self, new_price: u128);
    /// Get the DA mode for L1 Ethereum. This can be either Blob or Calldata, whichever is cheaper at the moment.
    fn get_da_mode(&self) -> L1DataAvailabilityMode;

    // fn get_pending_l1_handler_txs
}

#[async_trait::async_trait]
impl L1DataProvider for GasPriceProvider {
    async fn get_gas_prices(&self) -> GasPrices {
        (*self.gas_prices.lock().await).clone()
    }

    async fn set_gas_prices(&self, new_prices: GasPrices) {
        let mut prices = self.gas_prices.lock().await;
        *prices = new_prices;
    }

    async fn update_eth_l1_gas_price(&self, new_price: u128) {
        let mut prices = self.gas_prices.lock().await;
        prices.eth_l1_gas_price = new_price;
    }

    async fn update_eth_l1_data_gas_price(&self, new_price: u128) {
        let mut prices = self.gas_prices.lock().await;
        prices.eth_l1_data_gas_price = new_price;
    }

    async fn update_strk_l1_gas_price(&self, new_price: u128) {
        let mut prices = self.gas_prices.lock().await;
        prices.strk_l1_gas_price = new_price;
    }

    async fn update_strk_l1_data_gas_price(&self, new_price: u128) {
        let mut prices = self.gas_prices.lock().await;
        prices.strk_l1_data_gas_price = new_price;
    }

    fn get_da_mode(&self) -> L1DataAvailabilityMode {
        L1DataAvailabilityMode::Blob
    }
}
