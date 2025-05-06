//! TODO: this should be in the backend
use mp_block::header::{GasPrices, L1DataAvailabilityMode};
use mp_oracle::Oracle;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

#[derive(Clone)]
pub struct GasPriceProvider {
    /// Gas prices protected by a mutex
    gas_prices: Arc<Mutex<GasPrices>>,
    last_update: Arc<Mutex<SystemTime>>,
    /// Using Relaxed ordering for atomic operations since:
    /// 1. Gas prices are updated frequently (every few ms)
    /// 2. Slight inconsistencies in gas price visibility between threads are acceptable
    /// 3. Operations are independent and don't require synchronization with other memory operations
    /// 4. Provides minimal performance overhead compared to stricter ordering options
    gas_price_sync_enabled: Arc<AtomicBool>,
    data_gas_price_sync_enabled: Arc<AtomicBool>,
    strk_gas_price_sync_enabled: Arc<AtomicBool>,
    strk_data_gas_price_sync_enabled: Arc<AtomicBool>,
    pub oracle_provider: Option<Arc<dyn Oracle>>,
}

impl GasPriceProvider {
    pub fn new() -> Self {
        GasPriceProvider {
            gas_prices: Arc::new(Mutex::new(GasPrices::default())),
            last_update: Arc::new(Mutex::new(SystemTime::now())),
            gas_price_sync_enabled: Arc::new(AtomicBool::new(true)),
            data_gas_price_sync_enabled: Arc::new(AtomicBool::new(true)),
            strk_gas_price_sync_enabled: Arc::new(AtomicBool::new(true)),
            strk_data_gas_price_sync_enabled: Arc::new(AtomicBool::new(true)),
            oracle_provider: None,
        }
    }

    pub fn is_oracle_needed(&self) -> bool {
        self.gas_price_sync_enabled.load(Ordering::Relaxed)
            && (self.strk_gas_price_sync_enabled.load(Ordering::Relaxed)
                || self.strk_data_gas_price_sync_enabled.load(Ordering::Relaxed))
    }

    pub fn set_oracle_provider(&mut self, oracle_provider: impl Oracle + 'static) -> &mut Self {
        self.oracle_provider = Some(Arc::new(oracle_provider));
        self
    }

    pub fn set_gas_prices(&self, new_prices: GasPrices) {
        self.update_eth_l1_gas_price(new_prices.eth_l1_gas_price);
        self.update_strk_l1_gas_price(new_prices.strk_l1_gas_price);
        self.update_eth_l1_data_gas_price(new_prices.eth_l1_data_gas_price);
        self.update_strk_l1_data_gas_price(new_prices.strk_l1_data_gas_price);
    }

    pub fn set_gas_price_sync_enabled(&self, enabled: bool) {
        self.gas_price_sync_enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn set_data_gas_price_sync_enabled(&self, enabled: bool) {
        self.data_gas_price_sync_enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn set_strk_gas_price_sync_enabled(&self, enabled: bool) {
        self.strk_gas_price_sync_enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn set_strk_data_gas_price_sync_enabled(&self, enabled: bool) {
        self.strk_data_gas_price_sync_enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn update_last_update_timestamp(&self) {
        if self.gas_price_sync_enabled.load(Ordering::Relaxed)
            || self.data_gas_price_sync_enabled.load(Ordering::Relaxed)
        {
            let now = SystemTime::now();
            let mut timestamp = self.last_update.lock().unwrap();
            *timestamp = now;
        }
    }

    pub fn update_eth_l1_gas_price(&self, new_price: u128) {
        if self.gas_price_sync_enabled.load(Ordering::Relaxed) {
            let mut prices = self.gas_prices.lock().unwrap();
            prices.eth_l1_gas_price = new_price;
        }
    }

    pub fn update_eth_l1_data_gas_price(&self, new_price: u128) {
        if self.data_gas_price_sync_enabled.load(Ordering::Relaxed) {
            let mut prices = self.gas_prices.lock().unwrap();
            prices.eth_l1_data_gas_price = new_price;
        }
    }

    pub fn update_strk_l1_gas_price(&self, new_price: u128) {
        if self.strk_gas_price_sync_enabled.load(Ordering::Relaxed) {
            let mut prices = self.gas_prices.lock().unwrap();
            prices.strk_l1_gas_price = new_price;
        }
    }

    pub fn update_strk_l1_data_gas_price(&self, new_price: u128) {
        if self.strk_data_gas_price_sync_enabled.load(Ordering::Relaxed) {
            let mut prices = self.gas_prices.lock().unwrap();
            prices.strk_l1_data_gas_price = new_price;
        }
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
        L1DataAvailabilityMode::Calldata
    }
}
