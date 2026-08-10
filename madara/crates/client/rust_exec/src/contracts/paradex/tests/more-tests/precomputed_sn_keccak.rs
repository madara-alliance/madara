use crate::contracts::paradex::precomputed_sn_keccak::lookup_sn_keccak;

fn env_enabled() -> bool {
    let value = std::env::var("RUST_EXEC_PRECOMPUTED_SN_KECCAK").unwrap_or_default();
    if value.is_empty() {
        return false;
    }
    !matches!(value.to_ascii_lowercase().as_str(), "0" | "false" | "no" | "off")
}

#[test]
fn test_precomputed_sn_keccak_disabled_returns_none() {
    if env_enabled() {
        eprintln!("skipping: RUST_EXEC_PRECOMPUTED_SN_KECCAK enabled");
        return;
    }
    assert!(lookup_sn_keccak(b"ERC20_balances").is_none());
}

#[test]
fn test_precomputed_sn_keccak_hit_returns_value() {
    if !env_enabled() {
        eprintln!("skipping: RUST_EXEC_PRECOMPUTED_SN_KECCAK disabled");
        return;
    }
    let value = lookup_sn_keccak(b"ERC20_balances");
    assert!(value.is_some());
}

#[test]
fn test_precomputed_sn_keccak_contract_function_names_hit() {
    if !env_enabled() {
        eprintln!("skipping: RUST_EXEC_PRECOMPUTED_SN_KECCAK disabled");
        return;
    }

    assert!(lookup_sn_keccak(b"settle_trade_v3").is_some());
    assert!(lookup_sn_keccak(b"set_prices_and_funding_snapshot").is_some());
    assert!(lookup_sn_keccak(b"get_asset_kind").is_some());
}

#[test]
fn test_precomputed_sn_keccak_owner_name_lists_hit() {
    if !env_enabled() {
        eprintln!("skipping: RUST_EXEC_PRECOMPUTED_SN_KECCAK disabled");
        return;
    }

    for name in [
        b"ERC20_balances".as_slice(),
        b"Transfer",
        b"__validate_deploy__",
        b"Paraclear_account_fee_rate",
        b"PerpetualAssetBalanceUpdateV3",
        b"latest_updated_timestamp",
        b"market_fee_config",
    ] {
        assert!(lookup_sn_keccak(name).is_some());
    }
}

#[test]
fn test_precomputed_sn_keccak_miss_returns_none() {
    if !env_enabled() {
        eprintln!("skipping: RUST_EXEC_PRECOMPUTED_SN_KECCAK disabled");
        return;
    }
    let value = lookup_sn_keccak(b"not_in_map");
    assert!(value.is_none());
}

#[test]
fn test_precomputed_sn_keccak_non_utf8_returns_none() {
    if !env_enabled() {
        eprintln!("skipping: RUST_EXEC_PRECOMPUTED_SN_KECCAK disabled");
        return;
    }
    let bytes = [0xff, 0xfe, 0xfd];
    let value = lookup_sn_keccak(&bytes);
    assert!(value.is_none());
}
