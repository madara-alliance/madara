//! Type definitions for HeavyTradeSimulator contract.

use starknet_types_core::felt::Felt;

/// Account state structure (matches Cairo struct - 7 fields, no timestamp).
#[derive(Debug, Clone, Copy, Default)]
pub struct AccountState {
    pub balance: u128,
    pub perpetual_position: i128,
    pub realized_pnl: i128,
    pub total_fees_paid: u128,
    pub trade_count: u128,
    pub margin_ratio: u128, // Fixed-point 8 decimals
    pub is_active: bool,
}

/// Asset balance structure (2 fields, no timestamp).
#[derive(Debug, Clone, Copy, Default)]
pub struct AssetBalance {
    pub amount: u128,
    pub locked_amount: u128,
}

/// Fee configuration structure.
#[derive(Debug, Clone, Copy, Default)]
pub struct FeeConfig {
    pub maker_fee_rate: u128, // Basis points
    pub taker_fee_rate: u128,
    pub referral_rate: u128,
    pub insurance_fund_rate: u128,
}

/// Market state structure.
#[derive(Debug, Clone, Copy, Default)]
pub struct MarketState {
    pub total_volume: u128,
    pub open_interest: i128,
    pub last_price: u128,
    pub funding_rate: i128,
    pub is_enabled: bool,
}

// Constants matching Cairo contract
pub const MULTIPLIER: u128 = 100_000_000; // 8 decimals
pub const MAX_FEE_BPS: u128 = 1000; // 10%
pub const MIN_MARGIN_RATIO: u128 = 15_000_000; // 0.15 (15%)

impl AccountState {
    /// Deserialize from storage values (7 felts - no timestamp).
    pub fn from_storage(values: &[Felt]) -> Self {
        if values.len() < 7 {
            return Self::default();
        }
        Self {
            balance: felt_to_u128(values[0]),
            perpetual_position: felt_to_i128(values[1]),
            realized_pnl: felt_to_i128(values[2]),
            total_fees_paid: felt_to_u128(values[3]),
            trade_count: felt_to_u128(values[4]),
            margin_ratio: felt_to_u128(values[5]),
            is_active: values[6] != Felt::ZERO,
        }
    }

    /// Serialize to storage values (7 felts - no timestamp).
    pub fn to_storage(&self) -> Vec<Felt> {
        vec![
            Felt::from(self.balance),
            i128_to_felt(self.perpetual_position),
            i128_to_felt(self.realized_pnl),
            Felt::from(self.total_fees_paid),
            Felt::from(self.trade_count),
            Felt::from(self.margin_ratio),
            if self.is_active { Felt::ONE } else { Felt::ZERO },
        ]
    }
}

impl AssetBalance {
    /// Deserialize from storage values (2 felts - no timestamp).
    pub fn from_storage(values: &[Felt]) -> Self {
        if values.len() < 2 {
            return Self::default();
        }
        Self {
            amount: felt_to_u128(values[0]),
            locked_amount: felt_to_u128(values[1]),
        }
    }

    /// Serialize to storage values (2 felts - no timestamp).
    pub fn to_storage(&self) -> Vec<Felt> {
        vec![
            Felt::from(self.amount),
            Felt::from(self.locked_amount),
        ]
    }
}

impl FeeConfig {
    /// Deserialize from storage values (4 felts).
    pub fn from_storage(values: &[Felt]) -> Self {
        if values.len() < 4 {
            return Self::default();
        }
        Self {
            maker_fee_rate: felt_to_u128(values[0]),
            taker_fee_rate: felt_to_u128(values[1]),
            referral_rate: felt_to_u128(values[2]),
            insurance_fund_rate: felt_to_u128(values[3]),
        }
    }
}

impl MarketState {
    /// Deserialize from storage values (5 felts).
    pub fn from_storage(values: &[Felt]) -> Self {
        if values.len() < 5 {
            return Self::default();
        }
        Self {
            total_volume: felt_to_u128(values[0]),
            open_interest: felt_to_i128(values[1]),
            last_price: felt_to_u128(values[2]),
            funding_rate: felt_to_i128(values[3]),
            is_enabled: values[4] != Felt::ZERO,
        }
    }

    /// Serialize to storage values (5 felts).
    pub fn to_storage(&self) -> Vec<Felt> {
        vec![
            Felt::from(self.total_volume),
            i128_to_felt(self.open_interest),
            Felt::from(self.last_price),
            i128_to_felt(self.funding_rate),
            if self.is_enabled { Felt::ONE } else { Felt::ZERO },
        ]
    }
}

// Helper functions for type conversion
fn felt_to_u128(felt: Felt) -> u128 {
    let bytes = felt.to_bytes_be();
    let mut arr = [0u8; 16];
    arr.copy_from_slice(&bytes[16..32]);
    u128::from_be_bytes(arr)
}

fn felt_to_i128(felt: Felt) -> i128 {
    // For signed values, we treat the felt as a signed representation
    // Values >= 2^127 are negative (two's complement in felt space)
    let bytes = felt.to_bytes_be();
    let mut arr = [0u8; 16];
    arr.copy_from_slice(&bytes[16..32]);
    i128::from_be_bytes(arr)
}

fn i128_to_felt(value: i128) -> Felt {
    if value >= 0 {
        Felt::from(value as u128)
    } else {
        // For negative values, compute PRIME - |value|
        // This is the standard Cairo representation
        let abs_val = (-value) as u128;
        Felt::ZERO - Felt::from(abs_val)
    }
}

