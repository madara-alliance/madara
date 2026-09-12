use super::*;

fn next_u64(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 7;
    x ^= x >> 9;
    x ^= x << 8;
    *state = x;
    x
}

fn random_felt_251(state: &mut u64) -> Felt {
    let mut bytes = [0u8; 32];
    for chunk in bytes.chunks_mut(8) {
        chunk.copy_from_slice(&next_u64(state).to_be_bytes());
    }
    bytes[0] &= 0x07;
    Felt::from_bytes_be(&bytes)
}

#[test]
fn search_in_memory_mismatch_against_sequential_for_complex_storage_shapes() {
    for seed in 1_u64..=128 {
        let mut state = seed;
        let contracts: Vec<_> = (0..6).map(|_| random_felt_251(&mut state)).collect();

        let base_diff = StateDiff {
            storage_diffs: contracts
                .iter()
                .map(|address| ContractStorageDiffItem {
                    address: *address,
                    storage_entries: (0..8)
                        .map(|_| StorageEntry { key: random_felt_251(&mut state), value: random_felt_251(&mut state) })
                        .collect(),
                })
                .collect(),
            old_declared_contracts: vec![],
            declared_classes: vec![],
            deployed_contracts: contracts
                .iter()
                .map(|address| DeployedContractItem { address: *address, class_hash: random_felt_251(&mut state) })
                .collect(),
            replaced_classes: vec![],
            nonces: contracts
                .iter()
                .map(|address| NonceUpdate {
                    contract_address: *address,
                    nonce: Felt::from(next_u64(&mut state) & 0xff),
                })
                .collect(),
            migrated_compiled_classes: vec![],
        };
        let current_diff = StateDiff {
            storage_diffs: contracts
                .iter()
                .map(|address| ContractStorageDiffItem {
                    address: *address,
                    storage_entries: (0..6)
                        .map(|_| StorageEntry { key: random_felt_251(&mut state), value: random_felt_251(&mut state) })
                        .collect(),
                })
                .collect(),
            old_declared_contracts: vec![],
            declared_classes: vec![],
            deployed_contracts: vec![],
            replaced_classes: vec![],
            nonces: contracts
                .iter()
                .take(3)
                .map(|address| NonceUpdate {
                    contract_address: *address,
                    nonce: Felt::from(next_u64(&mut state) & 0xff),
                })
                .collect(),
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
            "seed {seed} produced a mismatch between in-memory and sequential trie computation"
        );
    }
}
