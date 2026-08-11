use std::collections::{BTreeMap, BTreeSet};

use blockifier::blockifier::transaction_executor::{TransactionExecutionOutput, TransactionExecutorError};
use blockifier::state::cached_state::StateMaps;
use blockifier::transaction::objects::TransactionExecutionInfo;
use mp_convert::{Felt, ToFelt};
use num_bigint::BigInt;
use num_traits::Zero;

const MAX_SAMPLES: usize = 25;

fn felt_hex(f: Felt) -> String {
    format!("{:#x}", f)
}

fn storage_key_hex(key: &starknet_api::state::StorageKey) -> String {
    let key_felt: Felt = (*key).into();
    felt_hex(key_felt)
}

fn storage_map(maps: &StateMaps) -> BTreeMap<(String, String), String> {
    let mut out = BTreeMap::new();
    for ((addr, key), value) in maps.storage.iter() {
        out.insert((felt_hex(addr.to_felt()), storage_key_hex(key)), felt_hex(*value));
    }
    out
}

fn nonces_map(maps: &StateMaps) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (addr, nonce) in maps.nonces.iter() {
        out.insert(felt_hex(addr.to_felt()), felt_hex(nonce.0));
    }
    out
}

fn class_hashes_map(maps: &StateMaps) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (addr, class_hash) in maps.class_hashes.iter() {
        out.insert(felt_hex(addr.to_felt()), felt_hex(class_hash.0));
    }
    out
}

fn compiled_class_hashes_map(maps: &StateMaps) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (class_hash, compiled) in maps.compiled_class_hashes.iter() {
        out.insert(felt_hex(class_hash.0), felt_hex(compiled.0));
    }
    out
}

fn declared_contracts_map(maps: &StateMaps) -> BTreeMap<String, bool> {
    let mut out = BTreeMap::new();
    for (class_hash, declared) in maps.declared_contracts.iter() {
        out.insert(felt_hex(class_hash.0), *declared);
    }
    out
}

fn diff_keys<K: Ord + Clone, V: PartialEq>(
    rust: &BTreeMap<K, V>,
    block: &BTreeMap<K, V>,
) -> (BTreeSet<K>, BTreeSet<K>, BTreeSet<K>) {
    let rust_keys: BTreeSet<K> = rust.keys().cloned().collect();
    let block_keys: BTreeSet<K> = block.keys().cloned().collect();

    let only_rust = rust_keys.difference(&block_keys).cloned().collect();
    let only_block = block_keys.difference(&rust_keys).cloned().collect();

    let mut mismatched = BTreeSet::new();
    for key in rust_keys.intersection(&block_keys) {
        if rust.get(key) != block.get(key) {
            mismatched.insert(key.clone());
        }
    }

    (only_rust, only_block, mismatched)
}

fn parse_hex_bigint(s: &str) -> Option<BigInt> {
    let trimmed = s.trim();
    let hex = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    BigInt::parse_bytes(hex.as_bytes(), 16)
}

#[allow(clippy::type_complexity)]
fn should_ignore_fee_mismatch(
    rust_maps: &StateMaps,
    block_maps: &StateMaps,
) -> Option<(String, (String, String, String, String), (String, String, String, String))> {
    let rust_storage = storage_map(rust_maps);
    let block_storage = storage_map(block_maps);
    let (only_rust_storage, only_block_storage, mismatched_storage) = diff_keys(&rust_storage, &block_storage);
    if !only_rust_storage.is_empty() || !only_block_storage.is_empty() {
        return None;
    }

    let rust_nonces = nonces_map(rust_maps);
    let block_nonces = nonces_map(block_maps);
    let (only_rust_nonces, only_block_nonces, mismatched_nonces) = diff_keys(&rust_nonces, &block_nonces);
    if !only_rust_nonces.is_empty() || !only_block_nonces.is_empty() || !mismatched_nonces.is_empty() {
        return None;
    }

    let rust_class_hashes = class_hashes_map(rust_maps);
    let block_class_hashes = class_hashes_map(block_maps);
    let (only_rust_class_hashes, only_block_class_hashes, mismatched_class_hashes) =
        diff_keys(&rust_class_hashes, &block_class_hashes);
    if !only_rust_class_hashes.is_empty() || !only_block_class_hashes.is_empty() || !mismatched_class_hashes.is_empty()
    {
        return None;
    }

    let rust_compiled = compiled_class_hashes_map(rust_maps);
    let block_compiled = compiled_class_hashes_map(block_maps);
    let (only_rust_compiled, only_block_compiled, mismatched_compiled) = diff_keys(&rust_compiled, &block_compiled);
    if !only_rust_compiled.is_empty() || !only_block_compiled.is_empty() || !mismatched_compiled.is_empty() {
        return None;
    }

    let rust_declared = declared_contracts_map(rust_maps);
    let block_declared = declared_contracts_map(block_maps);
    let (only_rust_declared, only_block_declared, mismatched_declared) = diff_keys(&rust_declared, &block_declared);
    if !only_rust_declared.is_empty() || !only_block_declared.is_empty() || !mismatched_declared.is_empty() {
        return None;
    }

    if mismatched_storage.len() != 2 {
        return None;
    }

    let mut it = mismatched_storage.iter();
    let (contract0, key0) = it.next().expect("len == 2").clone();
    let (contract1, key1) = it.next().expect("len == 2").clone();
    if contract0 != contract1 {
        return None;
    }

    let rust0 = rust_storage.get(&(contract0.clone(), key0.clone()))?.clone();
    let block0 = block_storage.get(&(contract0.clone(), key0.clone()))?.clone();
    let rust1 = rust_storage.get(&(contract1.clone(), key1.clone()))?.clone();
    let block1 = block_storage.get(&(contract1.clone(), key1.clone()))?.clone();

    let rust0_i = parse_hex_bigint(&rust0)?;
    let block0_i = parse_hex_bigint(&block0)?;
    let rust1_i = parse_hex_bigint(&rust1)?;
    let block1_i = parse_hex_bigint(&block1)?;

    let delta0 = rust0_i - block0_i;
    let delta1 = rust1_i - block1_i;

    if delta0.is_zero() || delta1.is_zero() {
        return None;
    }
    if &delta0 + &delta1 != BigInt::zero() {
        return None;
    }

    Some((contract0, (key0, rust0, block0, format!("{delta0}")), (key1, rust1, block1, format!("{delta1}"))))
}

fn log_detail_header(label: &str, only_rust: usize, only_block: usize, mismatched: usize) {
    tracing::info!(
        "TX_COMPARISON_DETAIL: {} only_in_rust={} only_in_blockifier={} value_mismatches={}",
        label,
        only_rust,
        only_block,
        mismatched
    );
}

fn log_storage_diff(rust_maps: &StateMaps, block_maps: &StateMaps) {
    let rust = storage_map(rust_maps);
    let block = storage_map(block_maps);
    let (only_rust, only_block, mismatched) = diff_keys(&rust, &block);
    log_detail_header("storage", only_rust.len(), only_block.len(), mismatched.len());

    for (i, (contract, key)) in only_rust.iter().take(MAX_SAMPLES).enumerate() {
        let value = rust.get(&(contract.clone(), key.clone())).expect("key exists");
        tracing::info!(
            "TX_COMPARISON_STORAGE_ONLY_IN_RUST idx={} contract={} key={} value={}",
            i,
            contract,
            key,
            value
        );
    }
    for (i, (contract, key)) in only_block.iter().take(MAX_SAMPLES).enumerate() {
        let value = block.get(&(contract.clone(), key.clone())).expect("key exists");
        tracing::info!(
            "TX_COMPARISON_STORAGE_ONLY_IN_BLOCKIFIER idx={} contract={} key={} value={}",
            i,
            contract,
            key,
            value
        );
    }
    for (i, (contract, key)) in mismatched.iter().take(MAX_SAMPLES).enumerate() {
        let rust_value = rust.get(&(contract.clone(), key.clone())).expect("key exists");
        let block_value = block.get(&(contract.clone(), key.clone())).expect("key exists");
        tracing::info!(
            "TX_COMPARISON_STORAGE_VALUE_MISMATCH idx={} contract={} key={} rust={} blockifier={}",
            i,
            contract,
            key,
            rust_value,
            block_value
        );
    }
}

fn log_simple_map_diff(
    label: &str,
    rust: &BTreeMap<String, String>,
    block: &BTreeMap<String, String>,
    only_rust_msg: &str,
    only_block_msg: &str,
    mismatch_msg: &str,
) {
    let (only_rust, only_block, mismatched) = diff_keys(rust, block);
    log_detail_header(label, only_rust.len(), only_block.len(), mismatched.len());

    for (i, k) in only_rust.iter().take(MAX_SAMPLES).enumerate() {
        let value = rust.get(k).expect("key exists");
        tracing::info!("{only_rust_msg} idx={i} key={k} value={value}");
    }
    for (i, k) in only_block.iter().take(MAX_SAMPLES).enumerate() {
        let value = block.get(k).expect("key exists");
        tracing::info!("{only_block_msg} idx={i} key={k} value={value}");
    }
    for (i, k) in mismatched.iter().take(MAX_SAMPLES).enumerate() {
        let rust_value = rust.get(k).expect("key exists");
        let block_value = block.get(k).expect("key exists");
        tracing::info!("{mismatch_msg} idx={i} key={k} rust={rust_value} blockifier={block_value}");
    }
}

fn log_declared_contracts_diff(rust_maps: &StateMaps, block_maps: &StateMaps) {
    let rust = declared_contracts_map(rust_maps);
    let block = declared_contracts_map(block_maps);
    let (only_rust, only_block, mismatched) = diff_keys(&rust, &block);
    log_detail_header("declared_contracts", only_rust.len(), only_block.len(), mismatched.len());

    for (i, k) in only_rust.iter().take(MAX_SAMPLES).enumerate() {
        let declared = rust.get(k).expect("key exists");
        tracing::info!("TX_COMPARISON_DECLARED_CONTRACT_ONLY_IN_RUST idx={} class_hash={} declared={}", i, k, declared);
    }
    for (i, k) in only_block.iter().take(MAX_SAMPLES).enumerate() {
        let declared = block.get(k).expect("key exists");
        tracing::info!(
            "TX_COMPARISON_DECLARED_CONTRACT_ONLY_IN_BLOCKIFIER idx={} class_hash={} declared={}",
            i,
            k,
            declared
        );
    }
    for (i, k) in mismatched.iter().take(MAX_SAMPLES).enumerate() {
        let rust_value = rust.get(k).expect("key exists");
        let block_value = block.get(k).expect("key exists");
        tracing::info!(
            "TX_COMPARISON_DECLARED_CONTRACT_VALUE_MISMATCH idx={} class_hash={} rust={} blockifier={}",
            i,
            k,
            rust_value,
            block_value
        );
    }
}

pub fn compare_tx_state_maps<RustErr: std::fmt::Display>(
    rust_formatted: &Result<(TransactionExecutionInfo, StateMaps), RustErr>,
    blockifier_first: Option<&Result<TransactionExecutionOutput, TransactionExecutorError>>,
) {
    let Some(blockifier_first) = blockifier_first else {
        tracing::info!("TX_COMPARISON: Does Not Match");
        tracing::info!("TX_COMPARISON_DETAIL: blockifier missing result");
        return;
    };

    let (Ok((_rust_info, rust_maps)), Ok((_block_info, block_maps))) = (rust_formatted, blockifier_first) else {
        tracing::info!("TX_COMPARISON: Does Not Match");
        if let Err(err) = rust_formatted {
            tracing::info!("TX_COMPARISON_DETAIL: rust formatted error: {err}");
        }
        if let Err(err) = blockifier_first {
            tracing::info!("TX_COMPARISON_DETAIL: blockifier error: {:#}", err);
        }
        return;
    };

    if rust_maps == block_maps {
        tracing::info!("TX_COMPARISON: Match");
        return;
    }

    if crate::config::ignore_fee_mismatch() {
        if let Some((contract, a, b)) = should_ignore_fee_mismatch(rust_maps, block_maps) {
            tracing::info!("TX_COMPARISON: Match");
            tracing::info!("TX_COMPARISON_DETAIL: ignored_fee_mismatch=true contract={} key_a={} rust_a={} blockifier_a={} delta_a={} key_b={} rust_b={} blockifier_b={} delta_b={}",
                contract,
                a.0,
                a.1,
                a.2,
                a.3,
                b.0,
                b.1,
                b.2,
                b.3
            );
            return;
        }
    }

    tracing::info!("TX_COMPARISON: Does Not Match");

    log_storage_diff(rust_maps, block_maps);
    log_simple_map_diff(
        "nonces",
        &nonces_map(rust_maps),
        &nonces_map(block_maps),
        "TX_COMPARISON_NONCE_ONLY_IN_RUST",
        "TX_COMPARISON_NONCE_ONLY_IN_BLOCKIFIER",
        "TX_COMPARISON_NONCE_VALUE_MISMATCH",
    );
    log_simple_map_diff(
        "class_hashes",
        &class_hashes_map(rust_maps),
        &class_hashes_map(block_maps),
        "TX_COMPARISON_CLASS_HASH_ONLY_IN_RUST",
        "TX_COMPARISON_CLASS_HASH_ONLY_IN_BLOCKIFIER",
        "TX_COMPARISON_CLASS_HASH_VALUE_MISMATCH",
    );
    log_simple_map_diff(
        "compiled_class_hashes",
        &compiled_class_hashes_map(rust_maps),
        &compiled_class_hashes_map(block_maps),
        "TX_COMPARISON_COMPILED_CLASS_HASH_ONLY_IN_RUST",
        "TX_COMPARISON_COMPILED_CLASS_HASH_ONLY_IN_BLOCKIFIER",
        "TX_COMPARISON_COMPILED_CLASS_HASH_VALUE_MISMATCH",
    );
    log_declared_contracts_diff(rust_maps, block_maps);
}
