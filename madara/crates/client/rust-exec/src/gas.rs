//! Gas tracking and fee calculation for Rust execution.
//!
//! This module provides gas tracking that mirrors Blockifier's gas accounting.

use starknet_types_core::felt::Felt;

use crate::types::{ContractAddress, StateDiff};

/// Gas costs for various operations (from Starknet versioned constants).
/// These values are approximate and should match the protocol version.
#[derive(Debug, Clone)]
pub struct GasCosts {
    /// Cost per storage read
    pub storage_read: u64,
    /// Cost per storage write (update existing)
    pub storage_write_update: u64,
    /// Cost per storage write (new key)
    pub storage_write_new: u64,
    /// Base cost per event
    pub event_base: u64,
    /// Cost per event key
    pub event_per_key: u64,
    /// Cost per event data element
    pub event_per_data: u64,
    /// Cost per L2→L1 message
    pub l2_to_l1_message: u64,
    /// Cost for contract call setup
    pub call_contract: u64,
    /// Entry point initial budget
    pub entry_point_initial_budget: u64,
    /// Cost per calldata element
    pub calldata_per_element: u64,
    /// Cost per byte of data availability
    pub da_per_byte: u64,
}

impl Default for GasCosts {
    fn default() -> Self {
        // Default gas costs matching Starknet 0.13.x / 0.14.x
        Self {
            storage_read: 100,
            storage_write_update: 200,
            storage_write_new: 500,
            event_base: 50,
            event_per_key: 10,
            event_per_data: 10,
            l2_to_l1_message: 500,
            call_contract: 100,
            entry_point_initial_budget: 100_000,
            calldata_per_element: 10,
            da_per_byte: 16, // Data availability cost
        }
    }
}

/// Tracks gas consumption during execution.
#[derive(Debug, Clone, Default)]
pub struct GasTracker {
    /// L1 gas consumed (for data availability)
    pub l1_gas: u64,
    /// L1 data gas (blob data)
    pub l1_data_gas: u64,
    /// L2 gas consumed (execution)
    pub l2_gas: u64,

    // Breakdown by category
    pub storage_read_gas: u64,
    pub storage_write_gas: u64,
    pub computation_gas: u64,
    pub event_gas: u64,
    pub message_gas: u64,
    pub calldata_gas: u64,

    /// Gas costs configuration
    costs: GasCosts,
}

impl GasTracker {
    /// Create a new gas tracker with default costs.
    pub fn new() -> Self {
        Self { costs: GasCosts::default(), ..Default::default() }
    }

    /// Create a new gas tracker with custom gas costs.
    pub fn with_costs(costs: GasCosts) -> Self {
        Self { costs, ..Default::default() }
    }

    /// Charge gas for a storage read operation.
    pub fn charge_storage_read(&mut self) {
        self.storage_read_gas += self.costs.storage_read;
        self.l2_gas += self.costs.storage_read;
    }

    /// Charge gas for a storage write operation.
    pub fn charge_storage_write(&mut self, is_new_key: bool) {
        let cost = if is_new_key { self.costs.storage_write_new } else { self.costs.storage_write_update };
        self.storage_write_gas += cost;
        self.l2_gas += cost;
    }

    /// Charge gas for emitting an event.
    pub fn charge_event(&mut self, num_keys: usize, num_data: usize) {
        let cost = self.costs.event_base
            + (num_keys as u64 * self.costs.event_per_key)
            + (num_data as u64 * self.costs.event_per_data);
        self.event_gas += cost;
        self.l2_gas += cost;
    }

    /// Charge gas for sending an L2→L1 message.
    pub fn charge_l2_to_l1_message(&mut self, payload_len: usize) {
        let cost = self.costs.l2_to_l1_message + (payload_len as u64 * self.costs.calldata_per_element);
        self.message_gas += cost;
        self.l2_gas += cost;
    }

    /// Charge gas for a contract call.
    pub fn charge_call_contract(&mut self) {
        self.computation_gas += self.costs.call_contract;
        self.l2_gas += self.costs.call_contract;
    }

    /// Charge gas for calldata.
    pub fn charge_calldata(&mut self, calldata_len: usize) {
        let cost = calldata_len as u64 * self.costs.calldata_per_element;
        self.calldata_gas += cost;
        self.l2_gas += cost;
    }

    /// Charge gas for computation (generic steps).
    pub fn charge_computation(&mut self, steps: u64) {
        self.computation_gas += steps;
        self.l2_gas += steps;
    }

    /// Calculate data availability gas from state diff.
    pub fn calculate_da_gas(&mut self, state_diff: &StateDiff) {
        let mut da_bytes = 0u64;

        // Each storage update: contract(32) + key(32) + value(32) = 96 bytes
        for (_contract, updates) in &state_diff.storage_updates {
            da_bytes += updates.len() as u64 * 96;
        }

        // Each nonce update: contract(32) + nonce(32) = 64 bytes
        da_bytes += state_diff.address_to_nonce.len() as u64 * 64;

        // Each class hash update: contract(32) + class_hash(32) = 64 bytes
        da_bytes += state_diff.address_to_class_hash.len() as u64 * 64;

        self.l1_data_gas = da_bytes * self.costs.da_per_byte;
    }

    /// Get total gas consumed.
    pub fn total_gas(&self) -> u64 {
        self.l2_gas
    }

    /// Get gas vector for fee calculation.
    pub fn gas_vector(&self) -> GasVector {
        GasVector { l1_gas: self.l1_gas, l1_data_gas: self.l1_data_gas, l2_gas: self.l2_gas }
    }
}

/// Gas vector for fee calculation.
#[derive(Debug, Clone, Default)]
pub struct GasVector {
    pub l1_gas: u64,
    pub l1_data_gas: u64,
    pub l2_gas: u64,
}

/// Block context containing gas prices and sequencer info.
#[derive(Debug, Clone)]
pub struct BlockContext {
    pub block_number: u64,
    pub block_timestamp: u64,
    pub sequencer_address: ContractAddress,

    // Gas prices in wei (for ETH fee token)
    pub l1_gas_price_wei: u128,
    pub l1_data_gas_price_wei: u128,
    pub l2_gas_price_wei: u128,

    // Gas prices in fri (for STRK fee token) - FRI = 10^-18 STRK
    pub l1_gas_price_fri: u128,
    pub l1_data_gas_price_fri: u128,
    pub l2_gas_price_fri: u128,

    // Fee token addresses
    pub eth_fee_token_address: ContractAddress,
    pub strk_fee_token_address: ContractAddress,
}

impl Default for BlockContext {
    fn default() -> Self {
        Self {
            block_number: 0,
            block_timestamp: 0,
            sequencer_address: ContractAddress(Felt::ZERO),
            l1_gas_price_wei: 1_000_000_000, // 1 gwei
            l1_data_gas_price_wei: 1_000_000_000,
            l2_gas_price_wei: 1_000_000,
            l1_gas_price_fri: 1_000_000_000_000, // 1000 gwei equivalent in FRI
            l1_data_gas_price_fri: 1_000_000_000_000,
            l2_gas_price_fri: 1_000_000_000,
            eth_fee_token_address: ContractAddress(Felt::ZERO),
            strk_fee_token_address: ContractAddress(Felt::ZERO),
        }
    }
}

/// Fee type (which token to use for fees).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeeType {
    Eth,
    Strk,
}

/// Calculate fee from gas vector and block context.
pub fn calculate_fee(gas: &GasVector, block_context: &BlockContext, fee_type: FeeType) -> u128 {
    let (l1_gas_price, l1_data_gas_price, l2_gas_price) = match fee_type {
        FeeType::Eth => {
            (block_context.l1_gas_price_wei, block_context.l1_data_gas_price_wei, block_context.l2_gas_price_wei)
        }
        FeeType::Strk => {
            (block_context.l1_gas_price_fri, block_context.l1_data_gas_price_fri, block_context.l2_gas_price_fri)
        }
    };

    let l1_fee = gas.l1_gas as u128 * l1_gas_price;
    let l1_data_fee = gas.l1_data_gas as u128 * l1_data_gas_price;
    let l2_fee = gas.l2_gas as u128 * l2_gas_price;

    l1_fee + l1_data_fee + l2_fee
}

/// Transaction resource bounds (max fees).
#[derive(Debug, Clone)]
pub struct ResourceBounds {
    pub max_l1_gas: u64,
    pub max_l1_gas_price: u128,
    pub max_l2_gas: u64,
    pub max_l2_gas_price: u128,
}

impl Default for ResourceBounds {
    fn default() -> Self {
        Self {
            max_l1_gas: 1_000_000,
            max_l1_gas_price: 100_000_000_000, // 100 gwei
            max_l2_gas: 10_000_000,
            max_l2_gas_price: 1_000_000_000,
        }
    }
}

/// Calculate max fee from resource bounds.
pub fn calculate_max_fee(bounds: &ResourceBounds) -> u128 {
    let l1_max = bounds.max_l1_gas as u128 * bounds.max_l1_gas_price;
    let l2_max = bounds.max_l2_gas as u128 * bounds.max_l2_gas_price;
    l1_max + l2_max
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gas_tracker_storage_operations() {
        let mut tracker = GasTracker::new();

        tracker.charge_storage_read();
        assert_eq!(tracker.storage_read_gas, 100);
        assert_eq!(tracker.l2_gas, 100);

        tracker.charge_storage_write(false);
        assert_eq!(tracker.storage_write_gas, 200);
        assert_eq!(tracker.l2_gas, 300);

        tracker.charge_storage_write(true);
        assert_eq!(tracker.storage_write_gas, 700); // 200 + 500
        assert_eq!(tracker.l2_gas, 800);
    }

    #[test]
    fn test_fee_calculation() {
        let gas = GasVector { l1_gas: 100, l1_data_gas: 200, l2_gas: 1000 };
        let block_context = BlockContext::default();

        let fee_eth = calculate_fee(&gas, &block_context, FeeType::Eth);
        assert!(fee_eth > 0);

        let fee_strk = calculate_fee(&gas, &block_context, FeeType::Strk);
        assert!(fee_strk > 0);
    }
}
