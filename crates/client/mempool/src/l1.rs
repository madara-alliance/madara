use dp_block::header::{GasPrices, L1DataAvailabilityMode};
use std::sync::{Arc, Mutex};

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

pub trait L1DataProvider: Send + Sync {
    fn get_gas_prices(&self) -> GasPrices;
    fn set_gas_prices(&self, new_prices: GasPrices);
    fn update_eth_l1_gas_price(&self, new_price: u128);
    fn update_eth_l1_data_gas_price(&self, new_price: u128);
    fn update_strk_l1_gas_price(&self, new_price: u128);
    fn update_strk_l1_data_gas_price(&self, new_price: u128);
    fn get_da_mode(&self) -> L1DataAvailabilityMode;
}

/// This trait enables the block production task to fill in the L1 info.
/// Gas prices and DA mode
impl L1DataProvider for GasPriceProvider {
    fn get_gas_prices(&self) -> GasPrices {
        self.gas_prices.lock().unwrap().clone()
    }

    fn set_gas_prices(&self, new_prices: GasPrices) {
        let mut prices = self.gas_prices.lock().unwrap();
        *prices = new_prices;
    }

    fn update_eth_l1_gas_price(&self, new_price: u128) {
        let mut prices = self.gas_prices.lock().unwrap();
        prices.eth_l1_gas_price = new_price;
    }

    fn update_eth_l1_data_gas_price(&self, new_price: u128) {
        let mut prices = self.gas_prices.lock().unwrap();
        prices.eth_l1_data_gas_price = new_price;
    }

    fn update_strk_l1_gas_price(&self, new_price: u128) {
        let mut prices = self.gas_prices.lock().unwrap();
        prices.strk_l1_gas_price = new_price;
    }

    fn update_strk_l1_data_gas_price(&self, new_price: u128) {
        let mut prices = self.gas_prices.lock().unwrap();
        prices.strk_l1_data_gas_price = new_price;
    }

    fn get_da_mode(&self) -> L1DataAvailabilityMode {
        L1DataAvailabilityMode::Blob
    }
}
