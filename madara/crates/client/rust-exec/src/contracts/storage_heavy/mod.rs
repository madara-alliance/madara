//! StorageHeavy contract implementation.
//!
//! This contract performs massive storage writes across multiple map types:
//! - 5 regular Map<u128, u128> (100 writes total)
//! - 2 nested Map<(u128, u128), u128> (50 writes total)
//! - 2 account-based maps (2 writes)
//! - 2 global counters (2 writes)
//!
//! Total: 154 storage writes + 9 events per call
//!
//! # Configuration
//!
//! The class hash is configured via environment variable:
//! ```bash
//! export RUST_EXEC_STORAGE_HEAVY_CLASS_HASH=0x0378fcc3ffae94edbf3eaee45bd22ce932adcf9d4b87836a42c61616dacc3cbc
//! ```

use once_cell::sync::Lazy;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Pedersen, StarkHash};

use crate::context::ExecutionContext;
use crate::contracts::erc20::layout::L2_ADDRESS_UPPER_BOUND;
use crate::contracts::ExecutionError;
use crate::state::StateReader;
use crate::storage::{function_selector, sn_keccak};
use crate::types::{ContractAddress, ExecutionResult};

pub mod functions;

// Storage variable base addresses (computed via sn_keccak)
pub static VALUE_MAP_1_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"value_map_1"));
pub static VALUE_MAP_2_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"value_map_2"));
pub static VALUE_MAP_3_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"value_map_3"));
pub static VALUE_MAP_4_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"value_map_4"));
pub static VALUE_MAP_5_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"value_map_5"));
pub static NESTED_MAP_1_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"nested_map_1"));
pub static NESTED_MAP_2_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"nested_map_2"));
pub static ACCOUNT_BALANCES_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"account_balances"));
pub static ACCOUNT_COUNTERS_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"account_counters"));
pub static TOTAL_WRITES_KEY: Lazy<Felt> = Lazy::new(|| sn_keccak(b"total_writes"));
pub static TOTAL_OPERATIONS_KEY: Lazy<Felt> = Lazy::new(|| sn_keccak(b"total_operations"));

// Event selectors
pub fn storage_write_batch_selector() -> Felt {
    sn_keccak(b"StorageWriteBatch")
}

pub fn operation_complete_selector() -> Felt {
    sn_keccak(b"OperationComplete")
}

// Helper: Compute storage key for Map<u128, u128>
// Matches starknet_api: Pedersen::hash(base, key) % L2_ADDRESS_UPPER_BOUND
pub fn map_u128_key(base: Felt, key: u128) -> Felt {
    let key_felt = Felt::from(key);
    let hash = Pedersen::hash(&base, &key_felt);
    hash.mod_floor(&L2_ADDRESS_UPPER_BOUND)
}

// Helper: Compute storage key for Map<(u128, u128), u128>
// For nested maps: hash(hash(base, key1), key2)
pub fn nested_map_key(base: Felt, key1: u128, key2: u128) -> Felt {
    let key1_felt = Felt::from(key1);
    let key2_felt = Felt::from(key2);

    // First hash: base + key1
    let hash1 = Pedersen::hash(&base, &key1_felt);
    // Second hash: hash1 + key2
    let hash2 = Pedersen::hash(&hash1, &key2_felt);
    hash2.mod_floor(&L2_ADDRESS_UPPER_BOUND)
}

// Helper: Compute storage key for Map<ContractAddress, T>
pub fn map_address_key(base: Felt, address: Felt) -> Felt {
    let hash = Pedersen::hash(&base, &address);
    hash.mod_floor(&L2_ADDRESS_UPPER_BOUND)
}

/// Name of the contract (for debugging/logging).
pub const NAME: &str = "StorageHeavy";

// Function selectors
fn massive_storage_write_selector() -> Felt {
    function_selector("massive_storage_write")
}

fn get_value_map1_selector() -> Felt {
    function_selector("get_value_map1")
}

fn get_value_map2_selector() -> Felt {
    function_selector("get_value_map2")
}

fn get_value_map3_selector() -> Felt {
    function_selector("get_value_map3")
}

fn get_total_writes_selector() -> Felt {
    function_selector("get_total_writes")
}

/// Check if this contract supports a given function selector.
pub fn supports_selector(selector: Felt) -> bool {
    selector == massive_storage_write_selector()
        || selector == get_value_map1_selector()
        || selector == get_value_map2_selector()
        || selector == get_value_map3_selector()
        || selector == get_total_writes_selector()
}

/// Get the human-readable function name for a selector.
pub fn get_function_name(selector: Felt) -> Option<String> {
    if selector == massive_storage_write_selector() {
        Some("massive_storage_write".to_string())
    } else if selector == get_value_map1_selector() {
        Some("get_value_map1".to_string())
    } else if selector == get_value_map2_selector() {
        Some("get_value_map2".to_string())
    } else if selector == get_value_map3_selector() {
        Some("get_value_map3".to_string())
    } else if selector == get_total_writes_selector() {
        Some("get_total_writes".to_string())
    } else {
        None
    }
}

/// Execute a function on the StorageHeavy contract.
pub fn execute<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    selector: Felt,
    calldata: &[Felt],
    caller: ContractAddress,
) -> Result<ExecutionResult, ExecutionError> {
    let mut ctx = ExecutionContext::new();

    if selector == massive_storage_write_selector() {
        functions::execute_massive_storage_write(state, contract, caller, calldata, &mut ctx)?;
    } else if selector == get_value_map1_selector() {
        functions::execute_get_value_map1(state, contract, calldata, &mut ctx)?;
    } else if selector == get_value_map2_selector() {
        functions::execute_get_value_map2(state, contract, calldata, &mut ctx)?;
    } else if selector == get_value_map3_selector() {
        functions::execute_get_value_map3(state, contract, calldata, &mut ctx)?;
    } else if selector == get_total_writes_selector() {
        functions::execute_get_total_writes(state, contract, &mut ctx)?;
    } else {
        return Err(ExecutionError::UnknownSelector(selector));
    }

    Ok(ctx.build_result())
}
