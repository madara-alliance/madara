//! ComprehensiveBenchmark contract implementation.
//!
//! This contract combines all benchmark patterns across 6 categories:
//! - A. Storage Benchmarks (6 functions)
//! - B. Hashing Benchmarks (3 functions)
//! - C. Math Benchmarks (8 functions)
//! - D. Control Flow Benchmarks (3 functions)
//! - E. Complex Pattern Benchmarks (5 functions)
//! - F. End-to-End Benchmarks (3 functions)
//!
//! Total: 28 benchmark functions + 3 getters

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

/// Human-readable contract name for logging
pub const NAME: &str = "ComprehensiveBenchmark";

// Storage variable base addresses (computed via sn_keccak)
pub static SIMPLE_COUNTER_KEY: Lazy<Felt> = Lazy::new(|| sn_keccak(b"simple_counter"));
pub static SIMPLE_ACCUMULATOR_KEY: Lazy<Felt> = Lazy::new(|| sn_keccak(b"simple_accumulator"));
pub static SIMPLE_MAP_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"simple_map"));
pub static NESTED_MAP_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"nested_map"));
pub static ADDRESS_MAP_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"address_map"));
pub static ACCOUNT_STATES_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"account_states"));
pub static POSITIONS_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"positions"));
pub static ACCOUNT_REFERRERS_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"account_referrers"));
pub static REFERRER_FEES_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"referrer_fees"));
pub static LAST_RESULT_U128_KEY: Lazy<Felt> = Lazy::new(|| sn_keccak(b"last_result_u128"));
pub static LAST_RESULT_I128_KEY: Lazy<Felt> = Lazy::new(|| sn_keccak(b"last_result_i128"));
pub static LAST_RESULT_FELT_KEY: Lazy<Felt> = Lazy::new(|| sn_keccak(b"last_result_felt"));
pub static TOTAL_OPERATIONS_KEY: Lazy<Felt> = Lazy::new(|| sn_keccak(b"total_operations"));
pub static TOTAL_HASHES_COMPUTED_KEY: Lazy<Felt> = Lazy::new(|| sn_keccak(b"total_hashes_computed"));
pub static TOTAL_EVENTS_EMITTED_KEY: Lazy<Felt> = Lazy::new(|| sn_keccak(b"total_events_emitted"));

// Event selectors
pub fn benchmark_complete_selector() -> Felt {
    sn_keccak(b"BenchmarkComplete")
}

pub fn operation_batch_selector() -> Felt {
    sn_keccak(b"OperationBatch")
}

pub fn hash_computed_selector() -> Felt {
    sn_keccak(b"HashComputed")
}

// Helper: Compute storage key for Map<u128, u128>
pub fn map_u128_key(base: Felt, key: u128) -> Felt {
    let key_felt = Felt::from(key);
    let hash = Pedersen::hash(&base, &key_felt);
    hash.mod_floor(&L2_ADDRESS_UPPER_BOUND)
}

// Helper: Compute storage key for Map<(u128, u128), u128>
pub fn nested_map_key(base: Felt, key1: u128, key2: u128) -> Felt {
    let key1_felt = Felt::from(key1);
    let key2_felt = Felt::from(key2);
    let hash1 = Pedersen::hash(&base, &key1_felt);
    let hash2 = Pedersen::hash(&hash1, &key2_felt);
    hash2.mod_floor(&L2_ADDRESS_UPPER_BOUND)
}

// Helper: Compute storage key for Map<ContractAddress, T>
pub fn map_address_key(base: Felt, address: Felt) -> Felt {
    let hash = Pedersen::hash(&base, &address);
    hash.mod_floor(&L2_ADDRESS_UPPER_BOUND)
}

// Helper: Compute storage key for Map<(ContractAddress, felt252), T>
pub fn map_address_felt_key(base: Felt, address: Felt, market_id: Felt) -> Felt {
    let hash1 = Pedersen::hash(&base, &address);
    let hash2 = Pedersen::hash(&hash1, &market_id);
    hash2.mod_floor(&L2_ADDRESS_UPPER_BOUND)
}

// Function selectors - A. Storage Benchmarks
fn benchmark_storage_read_selector() -> Felt {
    function_selector("benchmark_storage_read")
}

fn benchmark_storage_write_selector() -> Felt {
    function_selector("benchmark_storage_write")
}

fn benchmark_map_read_selector() -> Felt {
    function_selector("benchmark_map_read")
}

fn benchmark_map_write_selector() -> Felt {
    function_selector("benchmark_map_write")
}

fn benchmark_nested_map_read_selector() -> Felt {
    function_selector("benchmark_nested_map_read")
}

fn benchmark_nested_map_write_selector() -> Felt {
    function_selector("benchmark_nested_map_write")
}

// Function selectors - B. Hashing Benchmarks
fn benchmark_pedersen_hashing_selector() -> Felt {
    function_selector("benchmark_pedersen_hashing")
}

fn benchmark_pedersen_parallel_selector() -> Felt {
    function_selector("benchmark_pedersen_parallel")
}

fn benchmark_poseidon_hashing_selector() -> Felt {
    function_selector("benchmark_poseidon_hashing")
}

// Function selectors - C. Math Benchmarks
fn benchmark_math_division_selector() -> Felt {
    function_selector("benchmark_math_division")
}

fn benchmark_math_multiplication_selector() -> Felt {
    function_selector("benchmark_math_multiplication")
}

fn benchmark_math_addition_selector() -> Felt {
    function_selector("benchmark_math_addition")
}

fn benchmark_math_subtraction_selector() -> Felt {
    function_selector("benchmark_math_subtraction")
}

fn benchmark_math_signed_division_selector() -> Felt {
    function_selector("benchmark_math_signed_division")
}

fn benchmark_math_signed_multiplication_selector() -> Felt {
    function_selector("benchmark_math_signed_multiplication")
}

fn benchmark_math_combined_ops_selector() -> Felt {
    function_selector("benchmark_math_combined_ops")
}

fn benchmark_math_signed_combined_selector() -> Felt {
    function_selector("benchmark_math_signed_combined")
}

// Function selectors - D. Control Flow Benchmarks
fn benchmark_loops_selector() -> Felt {
    function_selector("benchmark_loops")
}

fn benchmark_conditionals_selector() -> Felt {
    function_selector("benchmark_conditionals")
}

fn benchmark_assertions_selector() -> Felt {
    function_selector("benchmark_assertions")
}

// Function selectors - E. Complex Pattern Benchmarks
fn benchmark_struct_read_selector() -> Felt {
    function_selector("benchmark_struct_read")
}

fn benchmark_struct_write_selector() -> Felt {
    function_selector("benchmark_struct_write")
}

fn benchmark_multi_map_coordination_selector() -> Felt {
    function_selector("benchmark_multi_map_coordination")
}

fn benchmark_referential_reads_selector() -> Felt {
    function_selector("benchmark_referential_reads")
}

fn benchmark_batch_operations_selector() -> Felt {
    function_selector("benchmark_batch_operations")
}

// Function selectors - F. End-to-End Benchmarks
fn benchmark_simple_transaction_selector() -> Felt {
    function_selector("benchmark_simple_transaction")
}

fn benchmark_medium_transaction_selector() -> Felt {
    function_selector("benchmark_medium_transaction")
}

fn benchmark_heavy_transaction_selector() -> Felt {
    function_selector("benchmark_heavy_transaction")
}

// Getters
fn get_last_result_u128_selector() -> Felt {
    function_selector("get_last_result_u128")
}

fn get_last_result_i128_selector() -> Felt {
    function_selector("get_last_result_i128")
}

fn get_total_operations_selector() -> Felt {
    function_selector("get_total_operations")
}

/// Check if this contract supports a given function selector.
pub fn supports_selector(selector: Felt) -> bool {
    // A. Storage Benchmarks
    selector == benchmark_storage_read_selector()
        || selector == benchmark_storage_write_selector()
        || selector == benchmark_map_read_selector()
        || selector == benchmark_map_write_selector()
        || selector == benchmark_nested_map_read_selector()
        || selector == benchmark_nested_map_write_selector()
        // B. Hashing Benchmarks
        || selector == benchmark_pedersen_hashing_selector()
        || selector == benchmark_pedersen_parallel_selector()
        || selector == benchmark_poseidon_hashing_selector()
        // C. Math Benchmarks
        || selector == benchmark_math_division_selector()
        || selector == benchmark_math_multiplication_selector()
        || selector == benchmark_math_addition_selector()
        || selector == benchmark_math_subtraction_selector()
        || selector == benchmark_math_signed_division_selector()
        || selector == benchmark_math_signed_multiplication_selector()
        || selector == benchmark_math_combined_ops_selector()
        || selector == benchmark_math_signed_combined_selector()
        // D. Control Flow Benchmarks
        || selector == benchmark_loops_selector()
        || selector == benchmark_conditionals_selector()
        || selector == benchmark_assertions_selector()
        // E. Complex Pattern Benchmarks
        || selector == benchmark_struct_read_selector()
        || selector == benchmark_struct_write_selector()
        || selector == benchmark_multi_map_coordination_selector()
        || selector == benchmark_referential_reads_selector()
        || selector == benchmark_batch_operations_selector()
        // F. End-to-End Benchmarks
        || selector == benchmark_simple_transaction_selector()
        || selector == benchmark_medium_transaction_selector()
        || selector == benchmark_heavy_transaction_selector()
        // Getters
        || selector == get_last_result_u128_selector()
        || selector == get_last_result_i128_selector()
        || selector == get_total_operations_selector()
}

/// Get the human-readable function name for a selector.
pub fn get_function_name(selector: Felt) -> Option<String> {
    if selector == benchmark_storage_read_selector() {
        Some("benchmark_storage_read".to_string())
    } else if selector == benchmark_storage_write_selector() {
        Some("benchmark_storage_write".to_string())
    } else if selector == benchmark_map_read_selector() {
        Some("benchmark_map_read".to_string())
    } else if selector == benchmark_map_write_selector() {
        Some("benchmark_map_write".to_string())
    } else if selector == benchmark_nested_map_read_selector() {
        Some("benchmark_nested_map_read".to_string())
    } else if selector == benchmark_nested_map_write_selector() {
        Some("benchmark_nested_map_write".to_string())
    } else if selector == benchmark_pedersen_hashing_selector() {
        Some("benchmark_pedersen_hashing".to_string())
    } else if selector == benchmark_pedersen_parallel_selector() {
        Some("benchmark_pedersen_parallel".to_string())
    } else if selector == benchmark_poseidon_hashing_selector() {
        Some("benchmark_poseidon_hashing".to_string())
    } else if selector == benchmark_math_division_selector() {
        Some("benchmark_math_division".to_string())
    } else if selector == benchmark_math_multiplication_selector() {
        Some("benchmark_math_multiplication".to_string())
    } else if selector == benchmark_math_addition_selector() {
        Some("benchmark_math_addition".to_string())
    } else if selector == benchmark_math_subtraction_selector() {
        Some("benchmark_math_subtraction".to_string())
    } else if selector == benchmark_math_signed_division_selector() {
        Some("benchmark_math_signed_division".to_string())
    } else if selector == benchmark_math_signed_multiplication_selector() {
        Some("benchmark_math_signed_multiplication".to_string())
    } else if selector == benchmark_math_combined_ops_selector() {
        Some("benchmark_math_combined_ops".to_string())
    } else if selector == benchmark_math_signed_combined_selector() {
        Some("benchmark_math_signed_combined".to_string())
    } else if selector == benchmark_loops_selector() {
        Some("benchmark_loops".to_string())
    } else if selector == benchmark_conditionals_selector() {
        Some("benchmark_conditionals".to_string())
    } else if selector == benchmark_assertions_selector() {
        Some("benchmark_assertions".to_string())
    } else if selector == benchmark_struct_read_selector() {
        Some("benchmark_struct_read".to_string())
    } else if selector == benchmark_struct_write_selector() {
        Some("benchmark_struct_write".to_string())
    } else if selector == benchmark_multi_map_coordination_selector() {
        Some("benchmark_multi_map_coordination".to_string())
    } else if selector == benchmark_referential_reads_selector() {
        Some("benchmark_referential_reads".to_string())
    } else if selector == benchmark_batch_operations_selector() {
        Some("benchmark_batch_operations".to_string())
    } else if selector == benchmark_simple_transaction_selector() {
        Some("benchmark_simple_transaction".to_string())
    } else if selector == benchmark_medium_transaction_selector() {
        Some("benchmark_medium_transaction".to_string())
    } else if selector == benchmark_heavy_transaction_selector() {
        Some("benchmark_heavy_transaction".to_string())
    } else if selector == get_last_result_u128_selector() {
        Some("get_last_result_u128".to_string())
    } else if selector == get_last_result_i128_selector() {
        Some("get_last_result_i128".to_string())
    } else if selector == get_total_operations_selector() {
        Some("get_total_operations".to_string())
    } else {
        None
    }
}

/// Execute a function on the ComprehensiveBenchmark contract.
pub fn execute<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    selector: Felt,
    calldata: &[Felt],
    caller: ContractAddress,
) -> Result<ExecutionResult, ExecutionError> {
    let mut ctx = ExecutionContext::new();

    // A. Storage Benchmarks
    if selector == benchmark_storage_read_selector() {
        functions::execute_benchmark_storage_read(state, contract, calldata, &mut ctx)?;
    } else if selector == benchmark_storage_write_selector() {
        functions::execute_benchmark_storage_write(state, contract, calldata, &mut ctx)?;
    } else if selector == benchmark_map_read_selector() {
        functions::execute_benchmark_map_read(state, contract, calldata, &mut ctx)?;
    } else if selector == benchmark_map_write_selector() {
        functions::execute_benchmark_map_write(state, contract, calldata, &mut ctx)?;
    } else if selector == benchmark_nested_map_read_selector() {
        functions::execute_benchmark_nested_map_read(state, contract, calldata, &mut ctx)?;
    } else if selector == benchmark_nested_map_write_selector() {
        functions::execute_benchmark_nested_map_write(state, contract, calldata, &mut ctx)?;
    }
    // B. Hashing Benchmarks
    else if selector == benchmark_pedersen_hashing_selector() {
        functions::execute_benchmark_pedersen_hashing(state, contract, calldata, &mut ctx)?;
    } else if selector == benchmark_pedersen_parallel_selector() {
        functions::execute_benchmark_pedersen_parallel(state, contract, calldata, &mut ctx)?;
    } else if selector == benchmark_poseidon_hashing_selector() {
        functions::execute_benchmark_poseidon_hashing(state, contract, calldata, &mut ctx)?;
    }
    // C. Math Benchmarks
    else if selector == benchmark_math_division_selector() {
        functions::execute_benchmark_math_division(state, contract, calldata, &mut ctx)?;
    } else if selector == benchmark_math_multiplication_selector() {
        functions::execute_benchmark_math_multiplication(state, contract, calldata, &mut ctx)?;
    } else if selector == benchmark_math_addition_selector() {
        functions::execute_benchmark_math_addition(state, contract, calldata, &mut ctx)?;
    } else if selector == benchmark_math_subtraction_selector() {
        functions::execute_benchmark_math_subtraction(state, contract, calldata, &mut ctx)?;
    } else if selector == benchmark_math_signed_division_selector() {
        functions::execute_benchmark_math_signed_division(state, contract, calldata, &mut ctx)?;
    } else if selector == benchmark_math_signed_multiplication_selector() {
        functions::execute_benchmark_math_signed_multiplication(state, contract, calldata, &mut ctx)?;
    } else if selector == benchmark_math_combined_ops_selector() {
        functions::execute_benchmark_math_combined_ops(state, contract, calldata, &mut ctx)?;
    } else if selector == benchmark_math_signed_combined_selector() {
        functions::execute_benchmark_math_signed_combined(state, contract, calldata, &mut ctx)?;
    }
    // D. Control Flow Benchmarks
    else if selector == benchmark_loops_selector() {
        functions::execute_benchmark_loops(state, contract, calldata, &mut ctx)?;
    } else if selector == benchmark_conditionals_selector() {
        functions::execute_benchmark_conditionals(state, contract, calldata, &mut ctx)?;
    } else if selector == benchmark_assertions_selector() {
        functions::execute_benchmark_assertions(state, contract, calldata, &mut ctx)?;
    }
    // E. Complex Pattern Benchmarks
    else if selector == benchmark_struct_read_selector() {
        functions::execute_benchmark_struct_read(state, contract, caller, calldata, &mut ctx)?;
    } else if selector == benchmark_struct_write_selector() {
        functions::execute_benchmark_struct_write(state, contract, caller, calldata, &mut ctx)?;
    } else if selector == benchmark_multi_map_coordination_selector() {
        functions::execute_benchmark_multi_map_coordination(state, contract, caller, calldata, &mut ctx)?;
    } else if selector == benchmark_referential_reads_selector() {
        functions::execute_benchmark_referential_reads(state, contract, caller, calldata, &mut ctx)?;
    } else if selector == benchmark_batch_operations_selector() {
        functions::execute_benchmark_batch_operations(state, contract, calldata, &mut ctx)?;
    }
    // F. End-to-End Benchmarks
    else if selector == benchmark_simple_transaction_selector() {
        functions::execute_benchmark_simple_transaction(state, contract, calldata, &mut ctx)?;
    } else if selector == benchmark_medium_transaction_selector() {
        functions::execute_benchmark_medium_transaction(state, contract, caller, calldata, &mut ctx)?;
    } else if selector == benchmark_heavy_transaction_selector() {
        functions::execute_benchmark_heavy_transaction(state, contract, caller, calldata, &mut ctx)?;
    }
    // Getters
    else if selector == get_last_result_u128_selector() {
        functions::execute_get_last_result_u128(state, contract, &mut ctx)?;
    } else if selector == get_last_result_i128_selector() {
        functions::execute_get_last_result_i128(state, contract, &mut ctx)?;
    } else if selector == get_total_operations_selector() {
        functions::execute_get_total_operations(state, contract, &mut ctx)?;
    } else {
        return Err(ExecutionError::UnknownSelector(selector));
    }

    Ok(ctx.build_result())
}
