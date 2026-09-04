use super::*;

#[test]
fn in_memory_single_block_root_matches_sequential_apply() {
    let backend_seq = setup_backend();
    let backend_mem = setup_backend();
    let diff = synthetic_state_diff(0);

    let expected_root = sequential_roots(&backend_seq, std::slice::from_ref(&diff))[0];
    let snapshot = fresh_snapshot(&backend_mem.db);
    let computed = compute_root_from_snapshot(&backend_mem.db, None, snapshot, 0, &diff, protocol_version(), false)
        .expect("compute");

    assert_eq!(computed.state_root, expected_root);
    assert!(computed.overlay.is_none(), "overlay should be absent when include_overlay=false");
}

#[test]
fn in_memory_single_block_root_preserves_existing_contract_storage() {
    let contract_address = Felt::from(777_u64);
    let class_hash = Felt::from(888_u64);

    let base_diff = StateDiff {
        storage_diffs: vec![ContractStorageDiffItem {
            address: contract_address,
            storage_entries: vec![
                StorageEntry { key: Felt::from(1_u64), value: Felt::from(10_u64) },
                StorageEntry { key: Felt::from(2_u64), value: Felt::from(20_u64) },
            ],
        }],
        old_declared_contracts: vec![],
        declared_classes: vec![],
        deployed_contracts: vec![DeployedContractItem { address: contract_address, class_hash }],
        replaced_classes: vec![],
        nonces: vec![NonceUpdate { contract_address, nonce: Felt::from(1_u64) }],
        migrated_compiled_classes: vec![],
    };
    let current_diff = StateDiff {
        storage_diffs: vec![ContractStorageDiffItem {
            address: contract_address,
            storage_entries: vec![
                StorageEntry { key: Felt::from(3_u64), value: Felt::from(30_u64) },
                StorageEntry { key: Felt::from(4_u64), value: Felt::from(40_u64) },
            ],
        }],
        old_declared_contracts: vec![],
        declared_classes: vec![],
        deployed_contracts: vec![],
        replaced_classes: vec![],
        nonces: vec![],
        migrated_compiled_classes: vec![],
    };

    let backend_expected = setup_backend();
    backend_expected.db.inner.state_apply_state_diff(0, &base_diff).expect("seed base history");
    let (expected_root, _timings) = backend_expected
        .write_access()
        .apply_to_global_trie(1, [&current_diff], protocol_version())
        .expect("sequential apply");

    let backend_actual = setup_backend();
    backend_actual.db.inner.state_apply_state_diff(0, &base_diff).expect("seed base history");
    let actual = compute_root_from_snapshot(
        &backend_actual.db,
        Some(0),
        fresh_snapshot(&backend_actual.db),
        1,
        &current_diff,
        protocol_version(),
        false,
    )
    .expect("actual compute");

    assert_eq!(actual.state_root, expected_root, "in-memory trie must preserve historical storage");
}

#[test]
fn in_memory_single_block_root_preserves_existing_storage_across_multiple_contracts() {
    let contract_a = Felt::from(777_u64);
    let contract_b = Felt::from(888_u64);
    let contract_c = Felt::from(999_u64);

    let base_diff = StateDiff {
        storage_diffs: vec![
            ContractStorageDiffItem {
                address: contract_a,
                storage_entries: vec![
                    StorageEntry { key: Felt::from(1_u64), value: Felt::from(10_u64) },
                    StorageEntry { key: Felt::from(2_u64), value: Felt::from(20_u64) },
                ],
            },
            ContractStorageDiffItem {
                address: contract_b,
                storage_entries: vec![
                    StorageEntry { key: Felt::from(3_u64), value: Felt::from(30_u64) },
                    StorageEntry { key: Felt::from(4_u64), value: Felt::from(40_u64) },
                ],
            },
            ContractStorageDiffItem {
                address: contract_c,
                storage_entries: vec![
                    StorageEntry { key: Felt::from(5_u64), value: Felt::from(50_u64) },
                    StorageEntry { key: Felt::from(6_u64), value: Felt::from(60_u64) },
                ],
            },
        ],
        old_declared_contracts: vec![],
        declared_classes: vec![],
        deployed_contracts: vec![
            DeployedContractItem { address: contract_a, class_hash: Felt::from(11_u64) },
            DeployedContractItem { address: contract_b, class_hash: Felt::from(22_u64) },
            DeployedContractItem { address: contract_c, class_hash: Felt::from(33_u64) },
        ],
        replaced_classes: vec![],
        nonces: vec![
            NonceUpdate { contract_address: contract_a, nonce: Felt::from(1_u64) },
            NonceUpdate { contract_address: contract_b, nonce: Felt::from(2_u64) },
            NonceUpdate { contract_address: contract_c, nonce: Felt::from(3_u64) },
        ],
        migrated_compiled_classes: vec![],
    };
    let current_diff = StateDiff {
        storage_diffs: vec![
            ContractStorageDiffItem {
                address: contract_a,
                storage_entries: vec![
                    StorageEntry { key: Felt::from(7_u64), value: Felt::from(70_u64) },
                    StorageEntry { key: Felt::from(8_u64), value: Felt::from(80_u64) },
                ],
            },
            ContractStorageDiffItem {
                address: contract_b,
                storage_entries: vec![
                    StorageEntry { key: Felt::from(9_u64), value: Felt::from(90_u64) },
                    StorageEntry { key: Felt::from(10_u64), value: Felt::from(100_u64) },
                ],
            },
            ContractStorageDiffItem {
                address: contract_c,
                storage_entries: vec![
                    StorageEntry { key: Felt::from(11_u64), value: Felt::from(110_u64) },
                    StorageEntry { key: Felt::from(12_u64), value: Felt::from(120_u64) },
                ],
            },
        ],
        old_declared_contracts: vec![],
        declared_classes: vec![],
        deployed_contracts: vec![],
        replaced_classes: vec![],
        nonces: vec![],
        migrated_compiled_classes: vec![],
    };

    let backend_expected = setup_backend();
    backend_expected.db.inner.state_apply_state_diff(0, &base_diff).expect("seed base history");
    let (expected_root, _timings) = backend_expected
        .write_access()
        .apply_to_global_trie(1, [&current_diff], protocol_version())
        .expect("sequential apply");

    let backend_actual = setup_backend();
    backend_actual.db.inner.state_apply_state_diff(0, &base_diff).expect("seed base history");
    let actual = compute_root_from_snapshot(
        &backend_actual.db,
        Some(0),
        fresh_snapshot(&backend_actual.db),
        1,
        &current_diff,
        protocol_version(),
        false,
    )
    .expect("actual compute");

    assert_eq!(
        actual.state_root, expected_root,
        "in-memory trie must preserve historical storage when multiple contract storage tries are updated together"
    );
}

const CONTRACT_2860_ADDRESS: &str = "0x286003f7c7bfc3f94e8f0af48b48302e7aee2fb13c23b141479ba00832ef2c6";
const CONTRACT_2860_CLASS_HASH: &str = "0x405f587ee8276e95a6466b37cad24e738ae0fcf2d56fffc94c26840d00a9833";
const CONTRACT_2860_BASE_ENTRIES: &[(&str, &str)] = &[
    ("0x1e2829f14592e9477a272088d9a12bfb3bcb689becc36ca875d36ebaa31cd93", "0xf71e88194323"),
    ("0x798816bc0d0cdd4d2ccbd7548fc58169b2200d880a5fdd20786697b56c829b0", "0xcc80e7c0"),
    ("0x53b6948612042f19eec772c614ec9d0f2577bf2eda0c6fc61c627618616ec88", "0x26f861fad69cd"),
    ("0xd18a1105b691ce8d68944784622bbabb7964b73d1ab81c0240ef2eda493f0f", "0x107c03fffbe4"),
    ("0x5b2ea3498b24afb618de9112ce585215a324afd78c6214c4087552b4dfbaf53", "0xbb64ab3501a9"),
    ("0x798816bc0d0cdd4d2ccbd7548fc58169b2200d880a5fdd20786697b56c829b1", "0x48b8cdcb87"),
    (
        "0xd18a1105b691ce8d68944784622bbabb7964b73d1ab81c0240ef2eda493f0e",
        "0x800000000000010ffffffffffffffffffffffffffffffffffffe547179ce483",
    ),
    ("0x224bea62583ba3072c880ababdcbe5d083407be91587ab7530899ba1455bf0", "0x15471b350e5dacf"),
    ("0x798816bc0d0cdd4d2ccbd7548fc58169b2200d880a5fdd20786697b56c829b2", "0x1af1ff9c4"),
    ("0x351fa422af4f1d90c3bd7681632c71b198df732bd8ccc704451ccb35d6d9234", "0xb9d6e0b380"),
    ("0x351fa422af4f1d90c3bd7681632c71b198df732bd8ccc704451ccb35d6d9235", "0x4d1289b62565"),
];
const CONTRACT_2860_CURRENT_ENTRIES: &[(&str, &str)] = &[
    (
        "0x1705a5bf42340e733815ddb6d42eabe16220907418e56599a6de3aed153ff15",
        "0x7251cef9a80c41e67176978fc08f3d649b56559a391c889b0719fd671e4d781",
    ),
    ("0x351fa422af4f1d90c3bd7681632c71b198df732bd8ccc704451ccb35d6d9235", "0x4d0d197fdefc"),
    (
        "0x5f7ffc919f3f967aeabf4a2b9aec07ff3fefadae6695dae4b3fcdd07be5439c",
        "0x6f373b346561036d98ea10fb3e60d2f459c872b1933b50b21fe6ef4fda3b75e",
    ),
    (
        "0x5f9e8f374b7853af89720bf3039ae9e7ede5cce41bf02b8efb3612de3248b5c",
        "0x6f373b346561036d98ea10fb3e60d2f459c872b1933b50b21fe6ef4fda3b75e",
    ),
    ("0x798816bc0d0cdd4d2ccbd7548fc58169b2200d880a5fdd20786697b56c829b2", "0x1af1d307e"),
    ("0x798816bc0d0cdd4d2ccbd7548fc58169b2200d880a5fdd20786697b56c829b1", "0x4d63217ed3"),
    ("0x5f7ffc919f3f967aeabf4a2b9aec07ff3fefadae6695dae4b3fcdd07be5439d", "0x9184e72a000"),
    ("0x1e2829f14592e9477a272088d9a12bfb3bcb689becc36ca875d36ebaa31cd93", "0xf71e8809f991"),
    ("0x351fa422af4f1d90c3bd7681632c71b198df732bd8ccc704451ccb35d6d9234", "0xb9c9c3c480"),
    (
        "0xd18a1105b691ce8d68944784622bbabb7964b73d1ab81c0240ef2eda493f0e",
        "0x800000000000010ffffffffffffffffffffffffffffffffffffe54651ba5166",
    ),
    ("0x798816bc0d0cdd4d2ccbd7548fc58169b2200d880a5fdd20786697b56c829b0", "0xd99dd6c0"),
    ("0x5b2ea3498b24afb618de9112ce585215a324afd78c6214c4087552b4dfbaf53", "0xbb64ab38105f"),
    ("0x224bea62583ba3072c880ababdcbe5d083407be91587ab7530899ba1455bf0", "0x15471b2f4422699"),
    (
        "0x1705a5bf42340e733815ddb6d42eabe16220907418e56599a6de3aed153ff16",
        "0x2f3f4d7c08fce6fe8ff30feb6a9dcbf88d2c787aeca673fca362eb2832dab72",
    ),
    (
        "0x2c11b97d01a63fdae176e288c5b93dc76d1274d50a5d570fe99ecdd14da6460",
        "0x7251cef9a80c41e67176978fc08f3d649b56559a391c889b0719fd671e4d781",
    ),
    ("0x53b6948612042f19eec772c614ec9d0f2577bf2eda0c6fc61c627618616ec88", "0x26f861fb9a4a9"),
    (
        "0x6a4a89c00e77bc38c421e7c89f665dd807f0315e361b9c0d5291fdb9918e6ca",
        "0x7251cef9a80c41e67176978fc08f3d649b56559a391c889b0719fd671e4d781",
    ),
    ("0xd18a1105b691ce8d68944784622bbabb7964b73d1ab81c0240ef2eda493f0f", "0x107c6d3edacb"),
];

/// Builds one captured contract-2860 diff from a stable key/value fixture table.
/// The deployment item is included only for the base block, matching the replay capture.
fn contract_2860_diff(entries: &[(&str, &str)], deploy: bool) -> StateDiff {
    let contract_address = Felt::from_hex_unchecked(CONTRACT_2860_ADDRESS);
    StateDiff {
        storage_diffs: vec![ContractStorageDiffItem {
            address: contract_address,
            storage_entries: entries
                .iter()
                .map(|(key, value)| StorageEntry {
                    key: Felt::from_hex_unchecked(key),
                    value: Felt::from_hex_unchecked(value),
                })
                .collect(),
        }],
        old_declared_contracts: vec![],
        declared_classes: vec![],
        deployed_contracts: deploy
            .then(|| DeployedContractItem {
                address: contract_address,
                class_hash: Felt::from_hex_unchecked(CONTRACT_2860_CLASS_HASH),
            })
            .into_iter()
            .collect(),
        replaced_classes: vec![],
        nonces: vec![],
        migrated_compiled_classes: vec![],
    }
}

/// Compares the in-memory root with sequential Bonsai for the captured contract-2860 workload.
/// The flag selects whether both paths begin from an already committed base trie snapshot.
fn assert_contract_2860_root_matches(persist_base_trie: bool) {
    let base_diff = contract_2860_diff(CONTRACT_2860_BASE_ENTRIES, true);
    let current_diff = contract_2860_diff(CONTRACT_2860_CURRENT_ENTRIES, false);
    let backend_expected = setup_backend();
    backend_expected.db.inner.state_apply_state_diff(0, &base_diff).expect("seed base history");
    if persist_base_trie {
        backend_expected
            .write_access()
            .apply_to_global_trie(0, [&base_diff], protocol_version())
            .expect("build persisted base trie");
    }
    let (expected_root, _) = backend_expected
        .write_access()
        .apply_to_global_trie(1, [&current_diff], protocol_version())
        .expect("sequential apply");

    let backend_actual = setup_backend();
    backend_actual.db.inner.state_apply_state_diff(0, &base_diff).expect("seed base history");
    if persist_base_trie {
        backend_actual
            .write_access()
            .apply_to_global_trie(0, [&base_diff], protocol_version())
            .expect("build persisted base trie");
    }
    let actual = compute_root_from_snapshot(
        &backend_actual.db,
        Some(0),
        fresh_snapshot(&backend_actual.db),
        1,
        &current_diff,
        protocol_version(),
        false,
    )
    .expect("compute captured root");
    assert_eq!(actual.state_root, expected_root);
}

#[test]
fn replay_regression_contract_2860_storage_root_matches_sequential_apply() {
    assert_contract_2860_root_matches(false);
}

#[test]
fn replay_regression_contract_2860_persisted_base_snapshot_matches_sequential_apply() {
    assert_contract_2860_root_matches(true);
}

#[test]
#[ignore = "requires captured /tmp state-update fixtures for blocks 669158 through 669160"]
fn debug_replay_regression_669160_full_window_matches_sequential_apply() {
    let base_diff = load_state_diff_fixture("/tmp/state_update_669158.json");
    let diff_159 = load_state_diff_fixture("/tmp/state_update_669159.json");
    let diff_160 = load_state_diff_fixture("/tmp/state_update_669160.json");

    let backend_expected = setup_backend();
    backend_expected.db.inner.state_apply_state_diff(0, &base_diff).expect("seed base history");
    backend_expected
        .write_access()
        .apply_to_global_trie(0, [&base_diff], protocol_version())
        .expect("build persisted base trie");
    backend_expected.db.inner.state_apply_state_diff(1, &diff_159).expect("seed 669159 history");
    backend_expected
        .write_access()
        .apply_to_global_trie(1, [&diff_159], protocol_version())
        .expect("sequential 669159 apply");
    backend_expected.db.inner.state_apply_state_diff(2, &diff_160).expect("seed 669160 history");
    let (expected_root, _timings) = backend_expected
        .write_access()
        .apply_to_global_trie(2, [&diff_160], protocol_version())
        .expect("sequential 669160 apply");

    let backend_actual = setup_backend();
    backend_actual.db.inner.state_apply_state_diff(0, &base_diff).expect("seed base history");
    backend_actual
        .write_access()
        .apply_to_global_trie(0, [&base_diff], protocol_version())
        .expect("build persisted base trie");

    let squashed = cumulative_squashed_state_diffs([&diff_159, &diff_160]);
    let cumulative = squashed.last().expect("cumulative diff for 669160");
    let actual = compute_root_from_snapshot(
        &backend_actual.db,
        Some(0),
        fresh_snapshot(&backend_actual.db),
        2,
        cumulative,
        protocol_version(),
        false,
    )
    .expect("actual compute from cumulative diff");

    assert_eq!(actual.state_root, expected_root, "669159+669160 cumulative replay window should match sequential");
}
