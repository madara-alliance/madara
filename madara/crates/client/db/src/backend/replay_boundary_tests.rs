use super::*;

fn boundary(block_n: u64) -> ReplayBlockBoundary {
    ReplayBlockBoundary { block_n, expected_tx_count: 0, last_tx_hash: Felt::ZERO }
}

#[test]
fn closed_replay_boundaries_have_bounded_retention() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
    for block_n in 0..(MAX_REPLAY_BOUNDARIES as u64 * 2) {
        backend.set_replay_boundary(boundary(block_n)).unwrap();
        assert!(backend.replay_boundary_mark_closed(block_n).unwrap().closed);
    }
    assert_eq!(backend.replay_boundaries.lock().unwrap().len(), MAX_REPLAY_BOUNDARIES);
    assert!(backend.get_replay_boundary_status(0).is_none());
    assert!(backend.get_replay_boundary_status(MAX_REPLAY_BOUNDARIES as u64).unwrap().closed);
}

#[test]
fn replay_boundary_capacity_preserves_active_work_and_allows_replacement() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
    for block_n in 0..MAX_REPLAY_BOUNDARIES as u64 {
        backend.set_replay_boundary(boundary(block_n)).unwrap();
    }
    let error = backend.set_replay_boundary(boundary(MAX_REPLAY_BOUNDARIES as u64)).unwrap_err();
    assert!(error.to_string().contains("capacity reached"));
    assert_eq!(backend.replay_boundaries.lock().unwrap().len(), MAX_REPLAY_BOUNDARIES);
    let mut replacement = boundary(0);
    replacement.expected_tx_count = 2;
    assert_eq!(backend.set_replay_boundary(replacement).unwrap().expected_tx_count, 2);
    assert!(backend.replay_boundary_exists(1));
}

#[test]
fn replay_boundary_eviction_skips_active_entries_and_reuses_closed_space() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
    for block_n in 0..MAX_REPLAY_BOUNDARIES as u64 {
        backend.set_replay_boundary(boundary(block_n)).unwrap();
    }
    backend.replay_boundary_mark_closed(1).unwrap();
    backend.set_replay_boundary(boundary(MAX_REPLAY_BOUNDARIES as u64)).unwrap();
    assert!(backend.replay_boundary_exists(0), "oldest active entry must survive");
    assert!(backend.get_replay_boundary_status(1).is_none());
    assert!(backend.replay_boundary_exists(MAX_REPLAY_BOUNDARIES as u64));
    assert_eq!(backend.replay_boundaries.lock().unwrap().len(), MAX_REPLAY_BOUNDARIES);
}
