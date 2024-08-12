use mp_block::header::{GasPrices, L1DataAvailabilityMode};

/// This trait enables the block production task to fill in the L1 info.
/// Gas prices and DA mode
pub trait L1DataProvider: Send + Sync {
    /// Get L1 data gas prices. This needs an oracle for STRK prices.
    fn get_gas_prices(&self) -> GasPrices;
    /// Get the DA mode for L1 Ethereum. This can be either Blob or Calldata, whichever is cheaper at the moment.
    fn get_da_mode(&self) -> L1DataAvailabilityMode;

    // fn get_pending_l1_handler_txs
}
