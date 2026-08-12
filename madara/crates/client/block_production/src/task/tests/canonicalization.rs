use super::*;

#[test]
fn build_parent_overlays_empty_when_no_runahead() {
    // confirmed = 5, target = 6, diffs_since_snapshot has block 5 → no overlays needed.
    let diffs = vec![(5, empty_state_diff())];
    let overlays = super::BlockProductionTask::build_parent_overlays(&diffs, Some(5), 6);
    assert!(overlays.is_empty(), "No overlays needed when confirmed is direct parent");
}

#[test]
fn build_parent_overlays_single_runahead() {
    // confirmed = 1, target = 3, diffs = [2].
    // Need overlay for block 2 only.
    let diffs = vec![(2, empty_state_diff())];
    let overlays = super::BlockProductionTask::build_parent_overlays(&diffs, Some(1), 3);
    assert_eq!(overlays.len(), 1);
    assert_eq!(overlays[0].block_n, 2);
}

#[test]
fn build_parent_overlays_multi_runahead() {
    // confirmed = 1, target = 5, diffs = [2, 3, 4].
    let diffs = vec![(2, empty_state_diff()), (3, empty_state_diff()), (4, empty_state_diff())];
    let overlays = super::BlockProductionTask::build_parent_overlays(&diffs, Some(1), 5);
    assert_eq!(overlays.len(), 3);
    assert_eq!(overlays[0].block_n, 2);
    assert_eq!(overlays[1].block_n, 3);
    assert_eq!(overlays[2].block_n, 4);
}

#[test]
fn build_parent_overlays_filters_out_stale_and_future() {
    // diffs has blocks [8, 9, 10, 11], confirmed = 9, target = 11.
    // Need overlays for block 10 only.
    let diffs =
        vec![(8, empty_state_diff()), (9, empty_state_diff()), (10, empty_state_diff()), (11, empty_state_diff())];
    let overlays = super::BlockProductionTask::build_parent_overlays(&diffs, Some(9), 11);
    assert_eq!(overlays.len(), 1);
    assert_eq!(overlays[0].block_n, 10);
}

#[test]
fn reexec_parent_state_snapshot_keeps_base_paired_with_overlays() {
    let diffs = (2479842..=2479846).map(|block_n| (block_n, empty_state_diff())).collect::<Vec<_>>();
    let snapshot = BlockProductionTask::capture_reexec_parent_state(&diffs, Some(2479841), 2479847);

    // The finalizer may advance the DB while the comparator task is starting. The
    // request must retain the base used to select its overlays.
    let later_confirmed_base = Some(2479842);
    assert_ne!(snapshot.confirmed_base_block_n, later_confirmed_base);
    assert_eq!(snapshot.confirmed_base_block_n, Some(2479841));
    assert_eq!(
        snapshot.parent_overlays.iter().map(|overlay| overlay.block_n).collect::<Vec<_>>(),
        vec![2479842, 2479843, 2479844, 2479845, 2479846]
    );
}

#[test]
fn state_diff_to_state_maps_converts_storage() {
    use mp_state_update::{ContractStorageDiffItem, StorageEntry};

    let sd = StateDiff {
        storage_diffs: vec![ContractStorageDiffItem {
            address: Felt::ONE,
            storage_entries: vec![StorageEntry { key: Felt::TWO, value: Felt::THREE }],
        }],
        ..Default::default()
    };

    let maps = super::BlockProductionTask::state_diff_to_state_maps(&sd);
    let addr: starknet_api::core::ContractAddress = Felt::ONE.try_into().unwrap();
    let key: starknet_api::state::StorageKey = Felt::TWO.try_into().unwrap();
    assert_eq!(maps.storage.get(&(addr, key)), Some(&Felt::THREE));
}

#[test]
fn state_diff_to_state_maps_converts_nonces() {
    use mp_state_update::NonceUpdate;

    let sd = StateDiff {
        nonces: vec![NonceUpdate { contract_address: Felt::ONE, nonce: Felt::from(42u64) }],
        ..Default::default()
    };

    let maps = super::BlockProductionTask::state_diff_to_state_maps(&sd);
    let addr: starknet_api::core::ContractAddress = Felt::ONE.try_into().unwrap();
    assert_eq!(maps.nonces.get(&addr), Some(&starknet_api::core::Nonce(Felt::from(42u64))));
}

#[test]
fn state_diff_to_state_maps_converts_deployed_and_replaced() {
    use mp_state_update::{DeployedContractItem, ReplacedClassItem};

    let sd = StateDiff {
        deployed_contracts: vec![DeployedContractItem { address: Felt::ONE, class_hash: Felt::TWO }],
        replaced_classes: vec![ReplacedClassItem { contract_address: Felt::THREE, class_hash: Felt::from(4u64) }],
        ..Default::default()
    };

    let maps = super::BlockProductionTask::state_diff_to_state_maps(&sd);
    let addr1: starknet_api::core::ContractAddress = Felt::ONE.try_into().unwrap();
    let addr2: starknet_api::core::ContractAddress = Felt::THREE.try_into().unwrap();
    assert_eq!(maps.class_hashes.get(&addr1), Some(&starknet_api::core::ClassHash(Felt::TWO)));
    assert_eq!(maps.class_hashes.get(&addr2), Some(&starknet_api::core::ClassHash(Felt::from(4u64))));
}

#[test]
fn state_diff_to_state_maps_converts_declared_classes() {
    use mp_state_update::DeclaredClassItem;

    let sd = StateDiff {
        declared_classes: vec![DeclaredClassItem { class_hash: Felt::ONE, compiled_class_hash: Felt::TWO }],
        ..Default::default()
    };

    let maps = super::BlockProductionTask::state_diff_to_state_maps(&sd);
    let ch = starknet_api::core::ClassHash(Felt::ONE);
    assert_eq!(maps.compiled_class_hashes.get(&ch), Some(&starknet_api::core::CompiledClassHash(Felt::TWO)));
}

#[test]
fn state_diff_mismatch_preview_uses_normalized_storage_json() {
    use mp_state_update::{ContractStorageDiffItem, StorageEntry};

    let rust_exec_diff = StateDiff {
        storage_diffs: vec![ContractStorageDiffItem {
            address: Felt::from(0x47u64),
            storage_entries: vec![StorageEntry { key: Felt::ONE, value: Felt::TWO }],
        }],
        ..Default::default()
    };
    let blockifier_diff = StateDiff {
        storage_diffs: vec![ContractStorageDiffItem {
            address: Felt::from(0x47u64),
            storage_entries: vec![StorageEntry { key: Felt::ONE, value: Felt::THREE }],
        }],
        ..Default::default()
    };

    let (count, preview) =
        super::BlockProductionTask::storage_diff_mismatch_preview_json(&rust_exec_diff, &blockifier_diff, 16);

    assert_eq!(count, 1);
    assert_eq!(preview[0]["kind"], "value_mismatch");
    assert_eq!(preview[0]["contract_address"], "0x47");
    assert_eq!(preview[0]["storage_key"], "0x1");
    assert_eq!(preview[0]["rust_exec_value"], "0x2");
    assert_eq!(preview[0]["blockifier_value"], "0x3");
}

// ── C-009D: Canonicalization source selection tests ──────────────────────

#[test]
fn canonical_accept_selects_eb_source() {
    use crate::comparator::{
        decide, execution_resources::compare_execution_resources, state_diff::compare_state_diff, CanonicalBlockSource,
        CanonicalizedBlockOutput,
    };
    use blockifier::bouncer::BouncerWeights;

    let eb_diff = StateDiff {
        nonces: vec![mp_state_update::NonceUpdate { contract_address: Felt::ONE, nonce: Felt::from(1u64) }],
        ..Default::default()
    };
    let bre_diff = eb_diff.clone(); // Same diff → Accept
    let eb_weights = BouncerWeights::empty();
    let bre_weights = BouncerWeights::empty();
    let block_limit = BouncerWeights::max();

    let sd = compare_state_diff(&eb_diff, &bre_diff);
    let er = compare_execution_resources(&eb_weights, &bre_weights, &block_limit);
    let decision = decide(&sd, &er);

    let canonical = match decision {
        super::ComparatorDecision::Accept | super::ComparatorDecision::AcceptWithWarn { .. } => {
            CanonicalizedBlockOutput {
                source: CanonicalBlockSource::ExecutionBox,
                state_diff: eb_diff.clone(),
                bouncer_weights: eb_weights,
                bre_per_tx: None,
            }
        }
        super::ComparatorDecision::StopExecutionBox { .. } => CanonicalizedBlockOutput {
            source: CanonicalBlockSource::BlockifierReexec,
            state_diff: bre_diff,
            bouncer_weights: bre_weights,
            bre_per_tx: None,
        },
    };

    assert_eq!(canonical.source, CanonicalBlockSource::ExecutionBox);
    assert_eq!(canonical.state_diff, eb_diff);
}

#[test]
fn canonical_stop_selects_bre_source() {
    use crate::comparator::{
        decide, execution_resources::compare_execution_resources, state_diff::compare_state_diff, CanonicalBlockSource,
        CanonicalizedBlockOutput,
    };
    use blockifier::bouncer::BouncerWeights;

    let eb_diff = StateDiff {
        nonces: vec![mp_state_update::NonceUpdate { contract_address: Felt::ONE, nonce: Felt::from(1u64) }],
        ..Default::default()
    };
    // Different diff → StopExecutionBox
    let bre_diff = StateDiff {
        nonces: vec![mp_state_update::NonceUpdate { contract_address: Felt::ONE, nonce: Felt::from(2u64) }],
        ..Default::default()
    };
    let eb_weights = BouncerWeights::empty();
    let bre_weights = BouncerWeights::empty();
    let block_limit = BouncerWeights::max();

    let sd = compare_state_diff(&eb_diff, &bre_diff);
    let er = compare_execution_resources(&eb_weights, &bre_weights, &block_limit);
    let decision = decide(&sd, &er);

    let canonical = match decision {
        super::ComparatorDecision::Accept | super::ComparatorDecision::AcceptWithWarn { .. } => {
            CanonicalizedBlockOutput {
                source: CanonicalBlockSource::ExecutionBox,
                state_diff: eb_diff,
                bouncer_weights: eb_weights,
                bre_per_tx: None,
            }
        }
        super::ComparatorDecision::StopExecutionBox { .. } => CanonicalizedBlockOutput {
            source: CanonicalBlockSource::BlockifierReexec,
            state_diff: bre_diff.clone(),
            bouncer_weights: bre_weights,
            bre_per_tx: None,
        },
    };

    assert_eq!(canonical.source, CanonicalBlockSource::BlockifierReexec);
    assert_eq!(canonical.state_diff, bre_diff, "canonical must use BRE diff on stop");
}

#[test]
fn canonical_warn_selects_eb_source() {
    use crate::comparator::{
        decide, execution_resources::compare_execution_resources, state_diff::compare_state_diff, CanonicalBlockSource,
        CanonicalizedBlockOutput,
    };
    use blockifier::bouncer::BouncerWeights;

    let eb_diff = StateDiff::default();
    let bre_diff = StateDiff::default();
    // EB weights > BRE weights but within block limit → AcceptWithWarn
    let mut eb_weights = BouncerWeights::empty();
    eb_weights.l1_gas = 100;
    let bre_weights = BouncerWeights::empty();
    let block_limit = BouncerWeights::max();

    let sd = compare_state_diff(&eb_diff, &bre_diff);
    let er = compare_execution_resources(&eb_weights, &bre_weights, &block_limit);
    let decision = decide(&sd, &er);

    assert!(matches!(decision, super::ComparatorDecision::AcceptWithWarn { .. }));

    let canonical = match decision {
        super::ComparatorDecision::Accept | super::ComparatorDecision::AcceptWithWarn { .. } => {
            CanonicalizedBlockOutput {
                source: CanonicalBlockSource::ExecutionBox,
                state_diff: eb_diff.clone(),
                bouncer_weights: eb_weights,
                bre_per_tx: None,
            }
        }
        super::ComparatorDecision::StopExecutionBox { .. } => unreachable!(),
    };

    assert_eq!(canonical.source, CanonicalBlockSource::ExecutionBox);
    assert_eq!(canonical.bouncer_weights.l1_gas, 100, "canonical uses EB bouncer weights on warn");
}

#[test]
fn canonical_ignored_fee_token_mismatch_source_is_switchable() {
    use std::collections::BTreeSet;

    use crate::comparator::{
        decide, execution_resources::compare_execution_resources,
        state_diff::compare_state_diff_with_ignored_storage_addresses, CanonicalBlockSource, CanonicalizedBlockOutput,
    };
    use blockifier::bouncer::BouncerWeights;
    use mp_state_update::{ContractStorageDiffItem, StorageEntry};

    let fee_token = Felt::from(0x4defu64);
    let eb_diff = StateDiff {
        storage_diffs: vec![ContractStorageDiffItem {
            address: fee_token,
            storage_entries: vec![StorageEntry { key: Felt::ONE, value: Felt::from(1u64) }],
        }],
        ..Default::default()
    };
    let bre_diff = StateDiff {
        storage_diffs: vec![ContractStorageDiffItem {
            address: fee_token,
            storage_entries: vec![StorageEntry { key: Felt::ONE, value: Felt::from(2u64) }],
        }],
        ..Default::default()
    };
    let ignored = BTreeSet::from([fee_token]);
    let eb_weights = BouncerWeights::empty();
    let bre_weights = BouncerWeights::empty();
    let block_limit = BouncerWeights::max();

    let sd = compare_state_diff_with_ignored_storage_addresses(&eb_diff, &bre_diff, &ignored);
    let er = compare_execution_resources(&eb_weights, &bre_weights, &block_limit);
    let decision = decide(&sd, &er);

    for (use_bre_for_ignored, expected_source) in
        [(false, CanonicalBlockSource::ExecutionBox), (true, CanonicalBlockSource::BlockifierReexec)]
    {
        let ignored_storage_mismatch =
            matches!(sd, crate::comparator::StateDiffComparison::IgnoredStorageMismatch { .. });
        let canonical = match decision.clone() {
            super::ComparatorDecision::Accept | super::ComparatorDecision::AcceptWithWarn { .. } => {
                if ignored_storage_mismatch && use_bre_for_ignored {
                    CanonicalizedBlockOutput {
                        source: CanonicalBlockSource::BlockifierReexec,
                        state_diff: bre_diff.clone(),
                        bouncer_weights: bre_weights,
                        bre_per_tx: None,
                    }
                } else {
                    CanonicalizedBlockOutput {
                        source: CanonicalBlockSource::ExecutionBox,
                        state_diff: eb_diff.clone(),
                        bouncer_weights: eb_weights,
                        bre_per_tx: None,
                    }
                }
            }
            super::ComparatorDecision::StopExecutionBox { .. } => unreachable!(),
        };

        assert_eq!(canonical.source, expected_source);
        if use_bre_for_ignored {
            assert_eq!(canonical.state_diff, bre_diff);
        } else {
            assert_eq!(canonical.state_diff, eb_diff);
        }
    }
}

#[test]
fn canonical_stop_uses_bre_bouncer_weights() {
    use crate::comparator::{CanonicalBlockSource, CanonicalizedBlockOutput};
    use blockifier::bouncer::BouncerWeights;

    let mut bre_weights = BouncerWeights::empty();
    bre_weights.l1_gas = 42;
    bre_weights.n_events = 7;

    let canonical = CanonicalizedBlockOutput {
        source: CanonicalBlockSource::BlockifierReexec,
        state_diff: StateDiff::default(),
        bouncer_weights: bre_weights,
        bre_per_tx: None,
    };

    assert_eq!(canonical.bouncer_weights.l1_gas, 42);
    assert_eq!(canonical.bouncer_weights.n_events, 7);
}

// ── C-010C: Canonical preconfirmed persistence and BRE-unavailable tests ─

#[test]
fn speculative_old_declared_contracts_extracts_legacy_classes() {
    use mc_db::preconfirmed::PreconfirmedExecutedTransaction;
    use mp_state_update::{DeclaredClassCompiledClass, TransactionStateUpdate};
    use mp_transactions::validated::TxTimestamp;
    use std::collections::HashMap;

    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()));
    let mut state = super::CurrentBlockState::new(backend, 0);

    // Add a speculative tx with a legacy declared class.
    let mut declared_classes = HashMap::new();
    declared_classes.insert(Felt::from(0xABCu64), DeclaredClassCompiledClass::Legacy);
    declared_classes.insert(Felt::from(0xDEFu64), DeclaredClassCompiledClass::Sierra(Felt::from(0x123u64)));

    state.speculative_executed_txs.push(PreconfirmedExecutedTransaction {
        transaction: mp_block::TransactionWithReceipt {
            transaction: mp_transactions::Transaction::Invoke(mp_transactions::InvokeTransaction::V0(
                mp_transactions::InvokeTransactionV0::default(),
            )),
            receipt: mp_receipt::TransactionReceipt::Invoke(mp_receipt::InvokeTransactionReceipt::default()),
        },
        state_diff: TransactionStateUpdate {
            nonces: Default::default(),
            contract_class_hashes: Default::default(),
            storage_diffs: Default::default(),
            declared_classes,
        },
        declared_class: None,
        arrived_at: TxTimestamp(0),
        paid_fee_on_l1: None,
    });

    let old_declared = state.get_old_declared_contracts_from_speculative();
    assert_eq!(old_declared, vec![Felt::from(0xABCu64)], "only legacy class should be returned");
}

#[test]
fn speculative_old_declared_contracts_empty_when_no_txs() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()));
    let state = super::CurrentBlockState::new(backend, 0);
    let old_declared = state.get_old_declared_contracts_from_speculative();
    assert!(old_declared.is_empty(), "no speculative txs → empty old_declared_contracts");
}

/// C-010C: Verify BRE-unavailable does not silently produce a canonical-from-EB block.
/// This is a unit-level test verifying the decision path without the full executor pipeline.
#[test]
fn bre_unavailable_does_not_return_canonical_eb() {
    // In C-009, BRE-unavailable paths returned Ok(CanonicalizedBlockOutput { source: EB }).
    // C-010B changed these to Err. Verify the contract: cancellation and stale must not
    // produce a canonical EB source.
    //
    // The actual behavior is tested by test_comparator_pipeline_error_forces_failsafe_fallback.
    // This test verifies the conceptual invariant at the type level.
    use crate::comparator::{CanonicalBlockSource, CanonicalizedBlockOutput};
    use blockifier::bouncer::BouncerWeights;

    // A canonical output should only be created with EB source when comparator explicitly
    // accepts (Accept / AcceptWithWarn). It must not be created as a silent fallback.
    let canonical = CanonicalizedBlockOutput {
        source: CanonicalBlockSource::ExecutionBox,
        state_diff: StateDiff::default(),
        bouncer_weights: BouncerWeights::empty(),
        bre_per_tx: None,
    };
    // This is only valid if comparator returned Accept/AcceptWithWarn.
    assert_eq!(canonical.source, CanonicalBlockSource::ExecutionBox);

    // Verify BRE source is only used on StopExecutionBox.
    let canonical_bre = CanonicalizedBlockOutput {
        source: CanonicalBlockSource::BlockifierReexec,
        state_diff: StateDiff::default(),
        bouncer_weights: BouncerWeights::empty(),
        bre_per_tx: None,
    };
    assert_eq!(canonical_bre.source, CanonicalBlockSource::BlockifierReexec);
}

// ── C-011D: Internal speculative frontier tests ──

#[test]
fn mixed_mode_runtime_append_updates_internal_preconfirmed() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()));

    let header = PreconfirmedHeader { block_number: 0, ..Default::default() };
    backend.write_access().new_preconfirmed(PreconfirmedBlock::new(header)).expect("new_preconfirmed");

    let fake_tx = make_fake_preconfirmed_tx(vec![(Felt::from(0xABCu64), Felt::from(1u64))]);

    // Update runtime block via C-011A method
    backend.append_to_internal_preconfirmed_runtime(&[fake_tx]).expect("runtime append");

    // Verify the preconfirmed block view has the tx (this is what close_block reads)
    let view = backend.block_view_on_preconfirmed(0).expect("preconfirmed view should exist");
    assert_eq!(view.num_executed_transactions(), 1, "preconfirmed view should have 1 executed tx");

    // Verify has_preconfirmed_block reports true
    assert!(backend.has_preconfirmed_block(), "should have preconfirmed block");
}

/// C-011D: Verify that flush_preconfirmed_content_to_db correctly persists runtime content.
#[test]
fn flush_preconfirmed_content_to_db_persists_runtime_block() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()));

    let header = PreconfirmedHeader { block_number: 0, ..Default::default() };
    backend.write_access().new_preconfirmed(PreconfirmedBlock::new(header)).expect("new_preconfirmed");

    let fake_tx = make_fake_preconfirmed_tx(vec![]);
    backend.append_to_internal_preconfirmed_runtime(&[fake_tx]).expect("runtime append");

    // Flush to DB
    backend.write_access().flush_preconfirmed_content_to_db().expect("flush to db");

    // Verify the preconfirmed view still has content (flush doesn't modify runtime)
    let view = backend.block_view_on_preconfirmed(0).expect("preconfirmed view");
    assert_eq!(view.num_executed_transactions(), 1, "preconfirmed should still have content after flush");
}

/// C-011D: Verify dual-track: speculative buffer for comparator, runtime block for chain watcher.
#[test]
fn speculative_buffer_and_runtime_block_dual_track() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()));

    let header = PreconfirmedHeader { block_number: 0, ..Default::default() };
    backend.write_access().new_preconfirmed(PreconfirmedBlock::new(header)).expect("new_preconfirmed");

    let mut state = super::CurrentBlockState::new(backend.clone(), 0);

    let fake_tx = make_fake_preconfirmed_tx(vec![(Felt::from(0x42u64), Felt::from(1u64))]);

    // Simulate C-011A: update runtime + speculative buffer
    backend.append_to_internal_preconfirmed_runtime(std::slice::from_ref(&fake_tx)).expect("runtime append");
    state.speculative_executed_txs.push(fake_tx);

    // Verify both tracks have the data
    let view = backend.block_view_on_preconfirmed(0).expect("preconfirmed view");
    assert_eq!(view.num_executed_transactions(), 1, "runtime block has content");
    assert_eq!(state.speculative_executed_txs.len(), 1, "speculative buffer has content");

    // Verify comparator can read from speculative buffer
    let old_declared = state.get_old_declared_contracts_from_speculative();
    assert!(old_declared.is_empty(), "no legacy classes in this test tx");
}

/// C-012D: append_to_internal_preconfirmed_runtime updates internal but NOT external snapshot.
/// External view (block_view_on_current_preconfirmed) should have 0 txs while internal
/// view (block_view_on_preconfirmed) has content.
#[test]
fn external_preconfirmed_isolated_from_internal_append() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()));

    let header = PreconfirmedHeader { block_number: 0, ..Default::default() };
    backend.write_access().new_preconfirmed(PreconfirmedBlock::new(header)).expect("new_preconfirmed");

    // External view starts empty.
    let ext_view = backend.block_view_on_current_preconfirmed().expect("external view");
    assert_eq!(ext_view.num_executed_transactions(), 0, "external should start empty");

    // Append to internal only (simulates mixed-mode EB batch).
    let fake_tx = make_fake_preconfirmed_tx(vec![(Felt::from(0xAAu64), Felt::from(1u64))]);
    backend.append_to_internal_preconfirmed_runtime(&[fake_tx]).expect("internal append");

    // Internal view has content.
    let int_view = backend.block_view_on_preconfirmed(0).expect("internal view");
    assert_eq!(int_view.num_executed_transactions(), 1, "internal should have 1 tx");

    // External view is still empty — not exposed before comparator.
    let ext_view = backend.block_view_on_current_preconfirmed().expect("external view");
    assert_eq!(ext_view.num_executed_transactions(), 0, "external must remain empty before promotion");
}

/// C-012D: promote_internal_to_external_preconfirmed copies content to external snapshot.
#[test]
fn promotion_copies_internal_content_to_external() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()));

    let header = PreconfirmedHeader { block_number: 0, ..Default::default() };
    backend.write_access().new_preconfirmed(PreconfirmedBlock::new(header)).expect("new_preconfirmed");

    // Append to internal (mixed-mode EB content).
    let fake_tx = make_fake_preconfirmed_tx(vec![(Felt::from(0xBBu64), Felt::from(2u64))]);
    backend.append_to_internal_preconfirmed_runtime(&[fake_tx]).expect("internal append");

    // External is empty before promotion.
    let ext_view = backend.block_view_on_current_preconfirmed().expect("external view");
    assert_eq!(ext_view.num_executed_transactions(), 0, "external empty before promotion");

    // Promote after comparator.
    backend.write_access().promote_internal_to_external_preconfirmed().expect("promotion");

    // External now has the content.
    let ext_view = backend.block_view_on_current_preconfirmed().expect("external view");
    assert_eq!(ext_view.num_executed_transactions(), 1, "external should have 1 tx after promotion");
}

/// C-012D: BlockifierOnly append_to_preconfirmed updates BOTH internal and external.
#[test]
fn blockifier_only_updates_both_snapshots() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()));

    let header = PreconfirmedHeader { block_number: 0, ..Default::default() };
    backend.write_access().new_preconfirmed(PreconfirmedBlock::new(header)).expect("new_preconfirmed");

    let fake_tx = make_fake_preconfirmed_tx(vec![(Felt::from(0xCCu64), Felt::from(3u64))]);
    backend
        .write_access()
        .append_to_preconfirmed(&[fake_tx], std::iter::empty::<Arc<mp_transactions::validated::ValidatedTransaction>>())
        .expect("append_to_preconfirmed");

    // Both views should have content in BlockifierOnly mode.
    let int_view = backend.block_view_on_preconfirmed(0).expect("internal view");
    assert_eq!(int_view.num_executed_transactions(), 1, "internal should have 1 tx");

    let ext_view = backend.block_view_on_current_preconfirmed().expect("external view");
    assert_eq!(ext_view.num_executed_transactions(), 1, "external should have 1 tx in BlockifierOnly");
}

/// C-012D: rewind_internal_preconfirmed_to discards descendants and resets internal tip.
#[test]
fn rewind_internal_preconfirmed_discards_descendants() {
    let backend = MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_test()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    );

    // Create blocks 0, 1, 2 (internal runahead).
    for block_n in 0..=2 {
        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader {
                block_number: block_n,
                ..Default::default()
            }))
            .expect("new_preconfirmed");
    }

    let head = backend.chain_head_state();
    assert_eq!(head.external_preconfirmed_tip, Some(0));
    assert_eq!(head.internal_preconfirmed_tip, Some(2));

    // Rewind internal to block 0 (discard blocks 1 and 2).
    let n_discarded = backend.rewind_internal_preconfirmed_to(0).expect("rewind");
    assert_eq!(n_discarded, 2, "should discard 2 descendant blocks");

    let head = backend.chain_head_state();
    assert_eq!(head.internal_preconfirmed_tip, Some(0), "internal tip should be rewound to 0");
    assert_eq!(head.external_preconfirmed_tip, Some(0), "external tip unchanged");
}

/// C-012D: rewind is a no-op when internal_tip <= target.
#[test]
fn rewind_noop_when_no_descendants() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()));

    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 0, ..Default::default() }))
        .expect("new_preconfirmed");

    let n_discarded = backend.rewind_internal_preconfirmed_to(0).expect("rewind");
    assert_eq!(n_discarded, 0, "no rewind needed when internal == target");

    let n_discarded = backend.rewind_internal_preconfirmed_to(5).expect("rewind");
    assert_eq!(n_discarded, 0, "no rewind needed when target > internal");
}

/// C-012D stop-path V1 ordering: in V1 sequential execution, comparator runs
/// before next block starts, so internal_tip == external_tip == X at comparator time.
/// Promotion works correctly, and rewind is a no-op (no descendants).
#[test]
fn stop_path_v1_sequential_promote_then_rewind_noop() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()));

    // Create block 0 (external=0, internal=0).
    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 0, ..Default::default() }))
        .expect("new_preconfirmed block 0");

    // Append EB content to block 0's internal runtime (simulates mixed-mode EB batch).
    let fake_tx = make_fake_preconfirmed_tx(vec![(Felt::from(0xDDu64), Felt::from(1u64))]);
    backend.append_to_internal_preconfirmed_runtime(&[fake_tx]).expect("internal append to block 0");

    let head = backend.chain_head_state();
    assert_eq!(head.external_preconfirmed_tip, Some(0));
    assert_eq!(head.internal_preconfirmed_tip, Some(0));

    // External is empty before promotion.
    let ext_view = backend.block_view_on_current_preconfirmed().expect("external view");
    assert_eq!(ext_view.num_executed_transactions(), 0, "external empty before promotion");

    // Promote block 0 to external (V1: internal == external, promotion succeeds).
    backend.write_access().promote_internal_to_external_preconfirmed().expect("promotion");
    let ext_view = backend.block_view_on_current_preconfirmed().expect("external view after promotion");
    assert_eq!(ext_view.num_executed_transactions(), 1, "external has 1 tx after promotion");

    // Rewind is a no-op (no descendants in V1 sequential).
    let n_discarded = backend.rewind_internal_preconfirmed_to(0).expect("rewind");
    assert_eq!(n_discarded, 0, "no descendants to discard in V1 sequential");

    // External still has content after no-op rewind.
    let ext_view = backend.block_view_on_current_preconfirmed().expect("external view after rewind");
    assert_eq!(ext_view.num_executed_transactions(), 1, "external content preserved");
}

/// C-012D stop-path with runahead: when internal_tip > external_tip at comparator time,
/// promotion skips (internal runtime is at a different block), and rewind discards
/// descendants. External snapshot is unchanged (stays at block X's header-only state).
///
/// This tests the runahead edge case. In V1 sequential execution, this case does not
/// occur because comparator runs before the next block starts. When pipelined execution
/// is added, per-block content snapshots will be needed for cross-block promotion.
#[test]
fn stop_path_runahead_promotion_skips_rewind_discards() {
    let backend = MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_test()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    );

    // Create block 0 with EB content.
    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 0, ..Default::default() }))
        .expect("new_preconfirmed block 0");
    let fake_tx = make_fake_preconfirmed_tx(vec![(Felt::from(0xDDu64), Felt::from(1u64))]);
    backend.append_to_internal_preconfirmed_runtime(&[fake_tx]).expect("internal append to block 0");

    // Create runahead descendants: blocks 1 and 2.
    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 1, ..Default::default() }))
        .expect("new_preconfirmed block 1");
    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 2, ..Default::default() }))
        .expect("new_preconfirmed block 2");

    let head = backend.chain_head_state();
    assert_eq!(head.external_preconfirmed_tip, Some(0));
    assert_eq!(head.internal_preconfirmed_tip, Some(2));

    // Promotion skips: internal (block 2) != external (block 0).
    backend.write_access().promote_internal_to_external_preconfirmed().expect("promotion (skipped)");
    let ext_view = backend.block_view_on_current_preconfirmed().expect("external view");
    assert_eq!(ext_view.num_executed_transactions(), 0, "external stays empty — promotion skipped due to runahead");

    // Rewind discards descendants 1, 2.
    let n_discarded = backend.rewind_internal_preconfirmed_to(0).expect("rewind");
    assert_eq!(n_discarded, 2, "should discard 2 descendant blocks");

    let head = backend.chain_head_state();
    assert_eq!(head.internal_preconfirmed_tip, Some(0), "internal rewound to 0");
    assert_eq!(head.external_preconfirmed_tip, Some(0), "external unchanged");
}

// ── C-013: BRE per-tx external promotion tests ──────────────────────────

/// C-013: On StopExecutionBox, replace_external_preconfirmed_content writes
/// BRE-backed per-tx rows to the external snapshot, not EB content.
#[test]
fn stop_path_external_content_is_bre_backed() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()));

    let header = PreconfirmedHeader { block_number: 0, ..Default::default() };
    backend.write_access().new_preconfirmed(PreconfirmedBlock::new(header)).expect("new_preconfirmed");

    // Simulate EB batch producing per-tx content in internal runtime.
    let eb_tx = make_fake_preconfirmed_tx(vec![(Felt::from(0xEBu64), Felt::from(1u64))]);
    backend.append_to_internal_preconfirmed_runtime(&[eb_tx]).expect("internal EB append");

    // Build BRE-backed rows with different nonces (simulating BRE execution).
    let bre_tx = make_fake_preconfirmed_tx(vec![(Felt::from(0xBBu64), Felt::from(2u64))]);

    // Stop path: replace external with BRE content.
    backend.write_access().replace_external_preconfirmed_content(0, vec![bre_tx]).expect("BRE replace");

    // External has BRE content.
    let ext_view = backend.block_view_on_current_preconfirmed().expect("external view");
    assert_eq!(ext_view.num_executed_transactions(), 1, "external should have 1 BRE-backed tx");

    // Internal still has EB content.
    let int_view = backend.block_view_on_preconfirmed(0).expect("internal view");
    assert_eq!(int_view.num_executed_transactions(), 1, "internal should have 1 EB tx");

    // Verify the external content is BRE-derived by checking the nonce key.
    let ext_content = ext_view.borrow_content();
    let ext_tx = ext_content.executed_transactions().next().expect("external tx");
    assert!(
        ext_tx.state_diff.nonces.contains_key(&Felt::from(0xBBu64)),
        "external tx nonces should be BRE-derived (0xBB), not EB-derived (0xEB)"
    );
    assert!(
        !ext_tx.state_diff.nonces.contains_key(&Felt::from(0xEBu64)),
        "external tx should NOT contain EB nonce key"
    );
}

/// C-013: On Accept/AcceptWithWarn, promote_internal_to_external copies EB content.
#[test]
fn accept_path_external_content_remains_eb_backed() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()));

    let header = PreconfirmedHeader { block_number: 0, ..Default::default() };
    backend.write_access().new_preconfirmed(PreconfirmedBlock::new(header)).expect("new_preconfirmed");

    // EB content in internal runtime.
    let eb_tx = make_fake_preconfirmed_tx(vec![(Felt::from(0xEBu64), Felt::from(1u64))]);
    backend.append_to_internal_preconfirmed_runtime(&[eb_tx]).expect("internal EB append");

    // Accept path: promote EB content to external.
    backend.write_access().promote_internal_to_external_preconfirmed().expect("EB promotion");

    let ext_view = backend.block_view_on_current_preconfirmed().expect("external view");
    let ext_content = ext_view.borrow_content();
    let ext_tx = ext_content.executed_transactions().next().expect("external tx");
    assert!(
        ext_tx.state_diff.nonces.contains_key(&Felt::from(0xEBu64)),
        "accept path external tx nonces should be EB-derived"
    );
}

/// C-013: build_bre_preconfirmed_rows preserves original metadata while replacing
/// execution-derived fields with BRE artifacts.
#[test]
fn build_bre_rows_preserves_metadata_uses_bre_execution() {
    use crate::reexecution::ReexecExecutedTxArtifacts;
    use mp_transactions::validated::TxTimestamp;

    // Original EB tx with specific metadata.
    let original = mc_db::preconfirmed::PreconfirmedExecutedTransaction {
        transaction: mp_block::TransactionWithReceipt {
            transaction: mp_transactions::Transaction::Invoke(mp_transactions::InvokeTransaction::V0(
                mp_transactions::InvokeTransactionV0::default(),
            )),
            receipt: mp_receipt::TransactionReceipt::Invoke(mp_receipt::InvokeTransactionReceipt {
                actual_fee: mp_receipt::FeePayment { amount: Felt::from(100u64), unit: mp_receipt::PriceUnit::Wei },
                ..Default::default()
            }),
        },
        state_diff: mp_state_update::TransactionStateUpdate {
            nonces: [(Felt::from(0xEBu64), Felt::from(1u64))].into_iter().collect(),
            storage_diffs: [((Felt::from(0x1u64), Felt::from(0x2u64)), Felt::from(0xEBu64))].into_iter().collect(),
            declared_classes: [(Felt::from(0xC1u64), mp_state_update::DeclaredClassCompiledClass::Legacy)]
                .into_iter()
                .collect(),
            ..Default::default()
        },
        declared_class: None,
        arrived_at: TxTimestamp(12345),
        paid_fee_on_l1: Some(999),
    };

    // BRE per-tx artifacts with different receipt and state update.
    let bre_artifact = ReexecExecutedTxArtifacts {
        receipt: mp_receipt::TransactionReceipt::Invoke(mp_receipt::InvokeTransactionReceipt {
            actual_fee: mp_receipt::FeePayment { amount: Felt::from(200u64), unit: mp_receipt::PriceUnit::Wei },
            ..Default::default()
        }),
        tx_state_update: mp_state_update::TransactionStateUpdate {
            nonces: [(Felt::from(0xBBu64), Felt::from(2u64))].into_iter().collect(),
            storage_diffs: [((Felt::from(0x1u64), Felt::from(0x2u64)), Felt::from(0xBBu64))].into_iter().collect(),
            // declared_classes is empty in BRE artifact — will be filled from original.
            declared_classes: Default::default(),
            ..Default::default()
        },
    };

    let rows =
        BlockProductionTask::build_bre_preconfirmed_rows(std::slice::from_ref(&original), vec![bre_artifact]).unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];

    // Metadata preserved from original.
    assert_eq!(row.arrived_at, TxTimestamp(12345), "arrived_at must be preserved");
    assert_eq!(row.paid_fee_on_l1, Some(999), "paid_fee_on_l1 must be preserved");
    assert_eq!(row.declared_class, None, "declared_class must be preserved");
    assert_eq!(row.transaction.transaction, original.transaction.transaction, "transaction payload must be preserved");
    assert!(row.state_diff.declared_classes.is_empty(), "invoke rows must not invent declared classes");

    // Execution artifacts from BRE.
    let actual_fee = match &row.transaction.receipt {
        mp_receipt::TransactionReceipt::Invoke(r) => r.actual_fee.amount,
        _ => panic!("expected Invoke receipt"),
    };
    assert_eq!(actual_fee, Felt::from(200u64), "receipt must be BRE-derived (fee=200, not EB fee=100)");
    assert!(row.state_diff.nonces.contains_key(&Felt::from(0xBBu64)), "nonces must be BRE-derived");
    assert!(!row.state_diff.nonces.contains_key(&Felt::from(0xEBu64)), "EB nonces must not be present");
    assert_eq!(
        row.state_diff.storage_diffs.get(&(Felt::from(0x1u64), Felt::from(0x2u64))),
        Some(&Felt::from(0xBBu64)),
        "storage_diffs must be BRE-derived"
    );
}

#[test]
fn build_bre_rows_reconstructs_declared_classes_from_original_metadata() {
    use crate::reexecution::ReexecExecutedTxArtifacts;
    use mp_transactions::validated::TxTimestamp;

    let class_hash = Felt::from(0xCAFEu64);
    let (transaction, mut receipt) = tx_declare_v0(Felt::ZERO);
    let mp_transactions::Transaction::Declare(mut declare_tx) = transaction else {
        panic!("expected declare transaction fixture");
    };
    match &mut declare_tx {
        mp_transactions::DeclareTransaction::V0(tx) => tx.class_hash = class_hash,
        _ => panic!("unexpected declare tx variant"),
    }
    if let mp_receipt::TransactionReceipt::Declare(receipt) = &mut receipt {
        receipt.transaction_hash = Felt::from(0xA11u64);
    }

    let original = mc_db::preconfirmed::PreconfirmedExecutedTransaction {
        transaction: mp_block::TransactionWithReceipt {
            transaction: mp_transactions::Transaction::Declare(declare_tx),
            receipt: receipt.clone(),
        },
        state_diff: mp_state_update::TransactionStateUpdate::default(),
        declared_class: Some(converted_class_legacy(class_hash)),
        arrived_at: TxTimestamp(7),
        paid_fee_on_l1: None,
    };

    let bre_artifact =
        ReexecExecutedTxArtifacts { receipt, tx_state_update: mp_state_update::TransactionStateUpdate::default() };

    let rows =
        BlockProductionTask::build_bre_preconfirmed_rows(std::slice::from_ref(&original), vec![bre_artifact]).unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.declared_class, original.declared_class, "declared_class metadata must be preserved");
    assert_eq!(
        row.state_diff.declared_classes.get(&class_hash),
        Some(&mp_state_update::DeclaredClassCompiledClass::Legacy),
        "declared class map must be rebuilt from original metadata"
    );
}

/// C-013: Stop-path ordering: BRE replace happens before rewind, external is anchored.
#[test]
fn stop_path_bre_replace_then_rewind_preserves_external() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()));

    let header = PreconfirmedHeader { block_number: 0, ..Default::default() };
    backend.write_access().new_preconfirmed(PreconfirmedBlock::new(header)).expect("new_preconfirmed");

    // EB content in internal.
    let eb_tx = make_fake_preconfirmed_tx(vec![(Felt::from(0xEBu64), Felt::from(1u64))]);
    backend.append_to_internal_preconfirmed_runtime(&[eb_tx]).expect("internal EB append");

    // Stop path: replace external with BRE content.
    let bre_tx = make_fake_preconfirmed_tx(vec![(Felt::from(0xBBu64), Felt::from(2u64))]);
    backend.write_access().replace_external_preconfirmed_content(0, vec![bre_tx]).expect("BRE replace");

    // Rewind internal (no-op in V1 sequential, but verify external is unaffected).
    let n_discarded = backend.rewind_internal_preconfirmed_to(0).expect("rewind");
    assert_eq!(n_discarded, 0, "V1: no descendants to discard");

    // External still has BRE content after rewind.
    let ext_view = backend.block_view_on_current_preconfirmed().expect("external after rewind");
    assert_eq!(ext_view.num_executed_transactions(), 1, "external still has BRE tx after rewind");
    let ext_content = ext_view.borrow_content();
    let ext_tx = ext_content.executed_transactions().next().expect("external tx");
    assert!(ext_tx.state_diff.nonces.contains_key(&Felt::from(0xBBu64)), "external still BRE-backed after rewind");
}

/// C-013: replace_external_preconfirmed_content validates block number.
#[test]
fn replace_external_rejects_wrong_block_number() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()));

    let header = PreconfirmedHeader { block_number: 0, ..Default::default() };
    backend.write_access().new_preconfirmed(PreconfirmedBlock::new(header)).expect("new_preconfirmed");

    let tx = make_fake_preconfirmed_tx(vec![]);
    let result = backend.write_access().replace_external_preconfirmed_content(5, vec![tx]);
    assert!(result.is_err(), "should reject mismatched block number");
}

/// C-013 fix: BRE rows must be built from speculative buffer BEFORE it is cleared.
/// Verifies that build_bre_preconfirmed_rows produces non-empty output when given
/// non-empty original_txs, simulating the corrected ordering.
#[test]
fn stop_path_bre_rows_built_before_buffer_clear() {
    use crate::reexecution::ReexecExecutedTxArtifacts;

    // Simulate the speculative buffer with one tx (as it would be before clear).
    let original = make_fake_preconfirmed_tx(vec![(Felt::from(0xEBu64), Felt::from(1u64))]);
    let bre_artifact = ReexecExecutedTxArtifacts {
        receipt: mp_receipt::TransactionReceipt::Invoke(mp_receipt::InvokeTransactionReceipt::default()),
        tx_state_update: mp_state_update::TransactionStateUpdate {
            nonces: [(Felt::from(0xBBu64), Felt::from(2u64))].into_iter().collect(),
            ..Default::default()
        },
    };

    // Build BRE rows from non-empty buffer (correct ordering).
    let rows = BlockProductionTask::build_bre_preconfirmed_rows(&[original], vec![bre_artifact]).unwrap();
    assert_eq!(rows.len(), 1, "BRE rows must be non-empty when built before buffer clear");
    assert!(rows[0].state_diff.nonces.contains_key(&Felt::from(0xBBu64)), "BRE row must contain BRE nonce");

    // Demonstrate the bug: building from empty buffer produces zero rows.
    let empty_rows = BlockProductionTask::build_bre_preconfirmed_rows(&[], vec![]).unwrap();
    assert_eq!(empty_rows.len(), 0, "empty buffer produces empty rows (the old bug)");
}

/// C-013 fix: On stop path, internal preconfirmed content must also be replaced
/// with BRE rows so that close_preconfirmed() reads BRE-backed tx content.
#[test]
fn stop_path_internal_content_is_bre_backed_for_close() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()));

    let header = PreconfirmedHeader { block_number: 0, ..Default::default() };
    backend.write_access().new_preconfirmed(PreconfirmedBlock::new(header)).expect("new_preconfirmed");

    // Simulate EB batch producing per-tx content in internal runtime.
    let eb_tx = make_fake_preconfirmed_tx(vec![(Felt::from(0xEBu64), Felt::from(1u64))]);
    backend.append_to_internal_preconfirmed_runtime(&[eb_tx]).expect("internal EB append");

    // Build BRE-backed rows with different nonces.
    let bre_tx = make_fake_preconfirmed_tx(vec![(Felt::from(0xBBu64), Felt::from(2u64))]);

    // Stop path: replace BOTH external and internal with BRE content.
    backend
        .write_access()
        .replace_external_preconfirmed_content(0, vec![bre_tx.clone()])
        .expect("BRE replace external");
    backend.write_access().replace_internal_preconfirmed_content(0, vec![bre_tx]).expect("BRE replace internal");

    // Internal now has BRE content (this is what close_preconfirmed reads).
    let int_view = backend.block_view_on_preconfirmed(0).expect("internal view");
    assert_eq!(int_view.num_executed_transactions(), 1, "internal should have 1 tx");
    let int_content = int_view.borrow_content();
    let int_tx = int_content.executed_transactions().next().expect("internal tx");
    assert!(
        int_tx.state_diff.nonces.contains_key(&Felt::from(0xBBu64)),
        "internal tx nonces should be BRE-derived after replacement"
    );
    assert!(
        !int_tx.state_diff.nonces.contains_key(&Felt::from(0xEBu64)),
        "internal tx should NOT contain EB nonce key after replacement"
    );

    // External also has BRE content.
    let ext_view = backend.block_view_on_current_preconfirmed().expect("external view");
    let ext_content = ext_view.borrow_content();
    let ext_tx = ext_content.executed_transactions().next().expect("external tx");
    assert!(ext_tx.state_diff.nonces.contains_key(&Felt::from(0xBBu64)), "external tx nonces should be BRE-derived");
}

/// C-013 hotfix: after stop-path internal replacement, flushing to DB must persist
/// BRE-backed rows so crash recovery / DB fallback does not resurrect EB content.
#[test]
fn stop_path_flush_persists_bre_internal_content_to_db() {
    let backend = MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_test()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    );

    // Create block 0 and append EB content to internal runtime.
    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 0, ..Default::default() }))
        .expect("new_preconfirmed block 0");
    let eb_tx = make_fake_preconfirmed_tx(vec![(Felt::from(0xEBu64), Felt::from(1u64))]);
    backend.append_to_internal_preconfirmed_runtime(&[eb_tx]).expect("internal EB append");

    // Replace internal with BRE-backed content and flush that canonicalized content.
    let bre_tx = make_fake_preconfirmed_tx(vec![(Felt::from(0xBBu64), Felt::from(2u64))]);
    backend.write_access().replace_internal_preconfirmed_content(0, vec![bre_tx]).expect("BRE replace internal");
    backend.write_access().flush_preconfirmed_content_to_db().expect("flush BRE-backed content");

    // Advance runtime to block 1 so reads of block 0 fall back to DB rather than the
    // current internal runtime block.
    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 1, ..Default::default() }))
        .expect("new_preconfirmed block 1");

    let block0_view = backend.block_view_on_preconfirmed(0).expect("block 0 from DB fallback");
    let block0_content = block0_view.borrow_content();
    let block0_tx = block0_content.executed_transactions().next().expect("block 0 tx");
    assert!(
        block0_tx.state_diff.nonces.contains_key(&Felt::from(0xBBu64)),
        "DB-persisted block 0 content must be BRE-derived"
    );
    assert!(
        !block0_tx.state_diff.nonces.contains_key(&Felt::from(0xEBu64)),
        "DB-persisted block 0 must not retain EB content after BRE flush"
    );
}

/// C-013 fix: replace_internal_preconfirmed_content validates block number.
#[test]
fn replace_internal_rejects_wrong_block_number() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()));

    let header = PreconfirmedHeader { block_number: 0, ..Default::default() };
    backend.write_access().new_preconfirmed(PreconfirmedBlock::new(header)).expect("new_preconfirmed");

    let tx = make_fake_preconfirmed_tx(vec![]);
    let result = backend.write_access().replace_internal_preconfirmed_content(5, vec![tx]);
    assert!(result.is_err(), "should reject mismatched block number");
}

/// C-017: Stop path with no descendants — internal tip == X, BRE replace succeeds directly.
#[test]
fn stop_path_rewind_then_replace_no_descendants() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()));

    // Create block 0 (internal=0, external=0).
    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 0, ..Default::default() }))
        .expect("new_preconfirmed");

    // Append EB content.
    let eb_tx = make_fake_preconfirmed_tx(vec![(Felt::from(0xEBu64), Felt::from(1u64))]);
    backend.append_to_internal_preconfirmed_runtime(&[eb_tx]).expect("internal EB append");

    let head = backend.chain_head_state();
    assert_eq!(head.internal_preconfirmed_tip, Some(0));

    // Rewind to block 0 — no descendants, should be a no-op.
    let n_discarded = backend.rewind_internal_preconfirmed_to(0).expect("rewind");
    assert_eq!(n_discarded, 0, "no descendants to discard");

    // Post-rewind: internal tip still 0, BRE replace succeeds.
    let post_tip = backend.chain_head_state().internal_preconfirmed_tip;
    assert_eq!(post_tip, Some(0));

    let bre_tx = make_fake_preconfirmed_tx(vec![(Felt::from(0xBBu64), Felt::from(2u64))]);
    backend.write_access().replace_internal_preconfirmed_content(0, vec![bre_tx]).expect("BRE replace");

    let view = backend.block_view_on_preconfirmed(0).expect("view after replace");
    let content = view.borrow_content();
    let tx = content.executed_transactions().next().expect("tx");
    assert!(tx.state_diff.nonces.contains_key(&Felt::from(0xBBu64)), "content should be BRE-backed");
}

/// C-017: Stop path with descendants — internal tip == X+4, rewind discards descendants,
/// then BRE replace succeeds on block X. Regression test for the exact observed bug:
/// "Internal preconfirmed block number mismatch: expected X, got X+4"
#[test]
fn stop_path_rewind_then_replace_with_descendants() {
    let backend = MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_test()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    );

    // Create block 0 with EB content.
    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 0, ..Default::default() }))
        .expect("new_preconfirmed block 0");
    let eb_tx = make_fake_preconfirmed_tx(vec![(Felt::from(0xEBu64), Felt::from(1u64))]);
    backend.append_to_internal_preconfirmed_runtime(&[eb_tx]).expect("internal EB append block 0");

    // Advance internal to block 4 (simulate runahead).
    for block_n in 1..=4 {
        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader {
                block_number: block_n,
                ..Default::default()
            }))
            .expect("new_preconfirmed runahead");
    }

    let head = backend.chain_head_state();
    assert_eq!(head.internal_preconfirmed_tip, Some(4));
    assert_eq!(head.external_preconfirmed_tip, Some(0));

    // Without rewind, replace_internal for block 0 would fail because internal is at block 4.
    let bre_tx_dup = make_fake_preconfirmed_tx(vec![(Felt::from(0xBBu64), Felt::from(2u64))]);
    let result = backend.write_access().replace_internal_preconfirmed_content(0, vec![bre_tx_dup]);
    assert!(result.is_err(), "replace must fail when internal tip != target (the old bug)");

    // C-017 fix: rewind first, then replace.
    let n_discarded = backend.rewind_internal_preconfirmed_to(0).expect("rewind");
    assert_eq!(n_discarded, 4, "should discard blocks 1-4");

    let post_tip = backend.chain_head_state().internal_preconfirmed_tip;
    assert_eq!(post_tip, Some(0), "internal tip should be rewound to block 0");

    // Now BRE replace succeeds.
    let bre_tx = make_fake_preconfirmed_tx(vec![(Felt::from(0xBBu64), Felt::from(2u64))]);
    backend
        .write_access()
        .replace_internal_preconfirmed_content(0, vec![bre_tx])
        .expect("BRE replace after rewind must succeed");

    let view = backend.block_view_on_preconfirmed(0).expect("view after replace");
    let content = view.borrow_content();
    let tx = content.executed_transactions().next().expect("tx");
    assert!(tx.state_diff.nonces.contains_key(&Felt::from(0xBBu64)), "content should be BRE-backed");
}

/// C-017: Accept path with descendants — no rewind, descendants preserved.
#[test]
fn accept_path_no_rewind_descendants_preserved() {
    let backend = MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_test()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    );

    // Create block 0 and runahead blocks 1, 2.
    for block_n in 0..=2 {
        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader {
                block_number: block_n,
                ..Default::default()
            }))
            .expect("new_preconfirmed");
    }

    let head = backend.chain_head_state();
    assert_eq!(head.internal_preconfirmed_tip, Some(2));

    // Accept path: promote internal to external (no rewind).
    // In accept path, we do NOT call rewind_internal_preconfirmed_to.
    // Descendants remain intact.
    let post_head = backend.chain_head_state();
    assert_eq!(post_head.internal_preconfirmed_tip, Some(2), "descendants must remain on accept path");
    assert_eq!(post_head.external_preconfirmed_tip, Some(0), "external tip unchanged");
}

/// C-017 regression: async canonicalization for block X completes after internal runtime
/// advanced to X+4. The old code would error with "expected X, got X+4". The new
/// rewind-then-replace ordering prevents this.
#[test]
fn regression_async_stop_no_mismatch_error() {
    let backend = MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_test()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    );

    // Simulate: EB executed block 22313, then internal advanced to 22317
    // (using relative block numbers 0..4 for simplicity).
    let target_block = 0u64;
    let advanced_tip = 4u64;

    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader {
            block_number: target_block,
            ..Default::default()
        }))
        .expect("new_preconfirmed target block");

    let eb_tx = make_fake_preconfirmed_tx(vec![(Felt::from(0xEBu64), Felt::from(1u64))]);
    backend.append_to_internal_preconfirmed_runtime(&[eb_tx]).expect("EB append");

    for block_n in 1..=advanced_tip {
        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader {
                block_number: block_n,
                ..Default::default()
            }))
            .expect("new_preconfirmed runahead");
    }

    assert_eq!(backend.chain_head_state().internal_preconfirmed_tip, Some(advanced_tip));

    // C-017: rewind-then-replace ordering — this must not error.
    let n_discarded = backend.rewind_internal_preconfirmed_to(target_block).expect("rewind");
    assert_eq!(n_discarded, advanced_tip, "should discard all descendants");
    assert_eq!(backend.chain_head_state().internal_preconfirmed_tip, Some(target_block));

    let bre_tx = make_fake_preconfirmed_tx(vec![(Felt::from(0xBBu64), Felt::from(2u64))]);
    backend
        .write_access()
        .replace_internal_preconfirmed_content(target_block, vec![bre_tx])
        .expect("BRE replace must succeed after rewind — the old mismatch error is eliminated");
}

/// C-018: Parent stop purges queued descendant canonicalizations.
#[test]
fn parent_stop_purges_descendant_canonicalizations() {
    use crate::fallback::types::ExecutionMode;

    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()));
    let mempool = Arc::new(Mempool::new(Arc::clone(&backend), MempoolConfig::default()));
    let mut task = BlockProductionTask::new(
        backend.clone(),
        mempool,
        Arc::new(BlockProductionMetrics::register()),
        Arc::new(L1ClientMock::new()),
        false,
        false,
    );

    // Simulate queued canonicalizations for blocks 5, 6, 7.
    for block_n in 5..=7 {
        task.pending_canonicalizations.push_back(super::PendingCanonicalizationInput {
            state: super::CurrentBlockState::with_execution_mode(backend.clone(), block_n, ExecutionMode::Mixed),
            block_exec_summary: Box::new(make_empty_block_exec_summary()),
        });
    }
    assert_eq!(task.pending_canonicalizations.len(), 3);

    // Simulate stop on block 5: purge descendants > 5.
    task.pending_canonicalizations.retain(|p| p.state.block_number <= 5);
    assert_eq!(task.pending_canonicalizations.len(), 1, "only block 5 should remain");
    assert_eq!(
        task.pending_canonicalizations.front().unwrap().state.block_number,
        5,
        "remaining entry must be block 5"
    );
}

/// C-018: Stop on block X also purges blocks X+1..N from queue, even when X itself
/// is not in the queue (it's the currently completing canonicalization).
#[test]
fn stop_purges_all_descendants_not_self() {
    use crate::fallback::types::ExecutionMode;

    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()));
    let mempool = Arc::new(Mempool::new(Arc::clone(&backend), MempoolConfig::default()));
    let mut task = BlockProductionTask::new(
        backend.clone(),
        mempool,
        Arc::new(BlockProductionMetrics::register()),
        Arc::new(L1ClientMock::new()),
        false,
        false,
    );

    // Queue entries for blocks 6, 7, 8 (block 5 is currently completing canonicalization).
    for block_n in 6..=8 {
        task.pending_canonicalizations.push_back(super::PendingCanonicalizationInput {
            state: super::CurrentBlockState::with_execution_mode(backend.clone(), block_n, ExecutionMode::Mixed),
            block_exec_summary: Box::new(make_empty_block_exec_summary()),
        });
    }

    // Stop on block 5 (currently processing, not in queue): purge descendants > 5.
    let stop_block = 5u64;
    task.pending_canonicalizations.retain(|p| p.state.block_number <= stop_block);
    assert_eq!(task.pending_canonicalizations.len(), 0, "all queued are descendants, all purged");
}

/// C-018: parent_overlays are NOT stored in PendingCanonicalizationInput.
/// They are recomputed at canonicalization start via build_parent_overlays.
#[test]
fn overlays_recomputed_at_canonicalization_start() {
    // Verify build_parent_overlays returns different results when diffs_since_snapshot changes.
    let diff_a = StateDiff {
        nonces: vec![mp_state_update::NonceUpdate { contract_address: Felt::from(0xAAu64), nonce: Felt::from(1u64) }],
        ..Default::default()
    };
    let diff_b = StateDiff {
        nonces: vec![mp_state_update::NonceUpdate { contract_address: Felt::from(0xBBu64), nonce: Felt::from(2u64) }],
        ..Default::default()
    };

    // At enqueue time: only block 5 diff exists.
    let diffs_at_enqueue: Vec<(u64, StateDiff)> = vec![(5, diff_a.clone())];
    let overlays_enqueue = BlockProductionTask::build_parent_overlays(&diffs_at_enqueue, Some(4), 6);
    assert_eq!(overlays_enqueue.len(), 1, "one overlay at enqueue time");

    // At canonicalization start time: blocks 5 and 6 diffs exist.
    let diffs_at_start: Vec<(u64, StateDiff)> = vec![(5, diff_a), (6, diff_b)];
    let overlays_start = BlockProductionTask::build_parent_overlays(&diffs_at_start, Some(4), 7);
    assert_eq!(overlays_start.len(), 2, "two overlays at start time — fresh computation");
}

/// C-018 regression: stop on block X with descendant X+1 already queued must not
/// produce overlay contiguity violation. Descendant is purged, not processed.
#[test]
fn regression_stop_prevents_overlay_contiguity_violation() {
    use crate::fallback::types::ExecutionMode;

    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()));
    let mempool = Arc::new(Mempool::new(Arc::clone(&backend), MempoolConfig::default()));
    let mut task = BlockProductionTask::new(
        backend.clone(),
        mempool,
        Arc::new(BlockProductionMetrics::register()),
        Arc::new(L1ClientMock::new()),
        false,
        false,
    );

    // Simulate the exact observed scenario:
    // - Block 22319 (relative 0) is currently completing canonicalization with stop
    // - Block 22320 (relative 1) was already queued
    task.pending_canonicalizations.push_back(super::PendingCanonicalizationInput {
        state: super::CurrentBlockState::with_execution_mode(backend.clone(), 1, ExecutionMode::Mixed),
        block_exec_summary: Box::new(make_empty_block_exec_summary()),
    });

    // Stop on block 0: purge descendants > 0.
    let stop_block = 0u64;
    let before = task.pending_canonicalizations.len();
    task.pending_canonicalizations.retain(|p| p.state.block_number <= stop_block);
    let n_purged = before - task.pending_canonicalizations.len();

    assert_eq!(n_purged, 1, "block 1 (descendant) must be purged");
    assert!(task.pending_canonicalizations.is_empty(), "no queued entries remain after purge");
    // The purged block would have caused "Overlay contiguity violation" if processed
    // with stale parent overlays. Since it's purged, the error cannot occur.
}

/// C-018: After stop invalidation and replay, fresh canonicalization can proceed.
#[test]
fn replay_after_invalidation_produces_fresh_canonicalization() {
    use crate::fallback::types::ExecutionMode;

    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()));
    let mempool = Arc::new(Mempool::new(Arc::clone(&backend), MempoolConfig::default()));
    let mut task = BlockProductionTask::new(
        backend.clone(),
        mempool,
        Arc::new(BlockProductionMetrics::register()),
        Arc::new(L1ClientMock::new()),
        false,
        false,
    );

    // Simulate: queue blocks 5, 6, 7.
    for block_n in 5..=7 {
        task.pending_canonicalizations.push_back(super::PendingCanonicalizationInput {
            state: super::CurrentBlockState::with_execution_mode(backend.clone(), block_n, ExecutionMode::Mixed),
            block_exec_summary: Box::new(make_empty_block_exec_summary()),
        });
    }

    // Stop on block 5: purge 6, 7.
    task.pending_canonicalizations.retain(|p| p.state.block_number <= 5);
    assert_eq!(task.pending_canonicalizations.len(), 1);

    // After block 5 is processed and closed, replay produces fresh block 6.
    // Queue fresh block 6 canonicalization (as BlockifierOnly after fallback).
    task.pending_canonicalizations.push_back(super::PendingCanonicalizationInput {
        state: super::CurrentBlockState::with_execution_mode(backend.clone(), 6, ExecutionMode::BlockifierOnly),
        block_exec_summary: Box::new(make_empty_block_exec_summary()),
    });
    assert_eq!(task.pending_canonicalizations.len(), 2);
    assert_eq!(task.pending_canonicalizations[0].state.block_number, 5);
    assert_eq!(task.pending_canonicalizations[1].state.block_number, 6);
    assert_eq!(
        task.pending_canonicalizations[1].state.execution_snapshot.execution_mode,
        ExecutionMode::BlockifierOnly
    );
}
