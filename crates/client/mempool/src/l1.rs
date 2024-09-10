use mp_block::header::{GasPrices, L1DataAvailabilityMode};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

#[derive(Clone)]
pub struct GasPriceProvider {
    gas_prices: Arc<Mutex<GasPrices>>,
    last_update: Arc<Mutex<SystemTime>>,
}

impl GasPriceProvider {
    pub fn new() -> Self {
        GasPriceProvider {
            gas_prices: Arc::new(Mutex::new(GasPrices::default())),
            last_update: Arc::new(Mutex::new(SystemTime::now())),
        }
    }

    pub fn set_gas_prices(&self, new_prices: GasPrices) {
        let mut prices = self.gas_prices.lock().unwrap();
        *prices = new_prices;
    }

    pub fn update_last_update_timestamp(&self) {
        let now = SystemTime::now();
        let mut timestamp = self.last_update.lock().unwrap();
        *timestamp = now;
    }

    pub fn update_eth_l1_gas_price(&self, new_price: u128) {
        let mut prices = self.gas_prices.lock().unwrap();
        prices.eth_l1_gas_price = new_price;
    }

    pub fn update_eth_l1_data_gas_price(&self, new_price: u128) {
        let mut prices = self.gas_prices.lock().unwrap();
        prices.eth_l1_data_gas_price = new_price;
    }

    pub fn update_strk_l1_gas_price(&self, new_price: u128) {
        let mut prices = self.gas_prices.lock().unwrap();
        prices.strk_l1_gas_price = new_price;
    }

    pub fn update_strk_l1_data_gas_price(&self, new_price: u128) {
        let mut prices = self.gas_prices.lock().unwrap();
        prices.strk_l1_data_gas_price = new_price;
    }
}

impl Default for GasPriceProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg_attr(any(test, feature = "testing"), mockall::automock)]
pub trait L1DataProvider: Send + Sync {
    fn get_gas_prices(&self) -> GasPrices;
    fn get_gas_prices_last_update(&self) -> SystemTime;
    fn get_da_mode(&self) -> L1DataAvailabilityMode;
}

/// This trait enables the block production task to fill in the L1 info.
/// Gas prices and DA mode
impl L1DataProvider for GasPriceProvider {
    fn get_gas_prices(&self) -> GasPrices {
        self.gas_prices.lock().unwrap().clone()
    }

    fn get_gas_prices_last_update(&self) -> SystemTime {
        *self.last_update.lock().expect("Failed to acquire lock")
    }

    fn get_da_mode(&self) -> L1DataAvailabilityMode {
        L1DataAvailabilityMode::Blob
    }
}
