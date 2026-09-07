use super::*;
use crate::MempoolConfig;
use mc_db::preconfirmed::{PreconfirmedBlock, PreconfirmedExecutedTransaction};
use mc_db::MadaraBackend;
use mp_block::{header::PreconfirmedHeader, TransactionWithReceipt};
use mp_receipt::{InvokeTransactionReceipt, TransactionReceipt};
use mp_state_update::{NonceUpdate, StateDiff, TransactionStateUpdate};
use mp_transactions::{InvokeTransaction, InvokeTransactionV3, Transaction};

async fn backend_with_genesis() -> Arc<MadaraBackend> {
    let backend = MadaraBackend::open_for_testing(Arc::new(mp_chain_config::ChainConfig::madara_test()));
    let mut genesis = mc_devnet::ChainGenesisDescription::base_config().unwrap();
    genesis.add_devnet_contracts(10).unwrap();
    genesis.build_and_store(&backend).await.unwrap();
    backend
}

fn executed(address: Felt, nonce: u64, hash: u64) -> PreconfirmedExecutedTransaction {
    PreconfirmedExecutedTransaction {
        transaction: TransactionWithReceipt {
            transaction: Transaction::Invoke(InvokeTransaction::V3(InvokeTransactionV3 {
                sender_address: address,
                nonce: Felt::from(nonce),
                ..Default::default()
            })),
            receipt: TransactionReceipt::Invoke(InvokeTransactionReceipt {
                transaction_hash: Felt::from(hash),
                ..Default::default()
            }),
        },
        state_diff: TransactionStateUpdate { nonces: [(address, Felt::from(nonce + 1))].into(), ..Default::default() },
        declared_class: None,
        arrived_at: mp_transactions::validated::TxTimestamp::now(),
        paid_fee_on_l1: None,
    }
}

fn append_block(backend: &Arc<MadaraBackend>, n: u64, txs: Vec<PreconfirmedExecutedTransaction>) {
    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new_with_content(
            PreconfirmedHeader { block_number: n, ..Default::default() },
            txs,
            vec![],
        ))
        .unwrap();
}

fn observe(
    mempool: &Mempool,
    state: &mut ChainWatcherState<mc_db::rocksdb::RocksDBStorage>,
    view: MadaraBlockView,
) -> ChainWatcherBranchEffects {
    let mut effects = ChainWatcherBranchEffects::new();
    mempool.handle_new_internal_frontier(state, view, &mut effects).unwrap();
    mempool.resolve_branch_effects(state, &mut effects).unwrap();
    effects
}

#[tokio::test]
async fn emptied_account_admission_uses_internal_execution_nonce() {
    let backend = backend_with_genesis().await;
    let mempool = Mempool::new(backend.clone(), MempoolConfig::default());
    let address = Felt::from(123u64);
    let contract_address = address.try_into().unwrap();
    let mut state = ChainWatcherState::new(Some(0));

    for nonce in 0..3 {
        let tx = executed(address, nonce, 100 + nonce);
        mempool.accept_tx(tx.to_validated()).await.unwrap();
        {
            let mut guard = mempool.inner.write().await;
            assert_eq!(guard.pop_next_ready().unwrap().hash, Felt::from(100 + nonce));
            assert!(guard.get_account_nonce(&contract_address).is_none());
        }
        append_block(&backend, nonce + 1, vec![tx]);
        let effects = observe(&mempool, &mut state, backend.block_view_on_preconfirmed(nonce + 1).unwrap().into());
        mempool.apply_nonce_updates(effects.nonce_updates, effects.nonce_update_mode).await.unwrap();
    }
    // The latest block need not touch this account; admission must inspect the earlier suffix too.
    append_block(&backend, 4, vec![]);
    assert_eq!(backend.view_on_latest().get_contract_nonce(&address).unwrap(), Some(Felt::ONE));
    assert!(matches!(
        mempool.accept_tx(executed(address, 1, 998).to_validated()).await,
        Err(crate::MempoolInsertionError::InnerMempool(crate::inner::TxInsertionError::NonceTooLow { .. }))
    ));
    mempool.accept_tx(executed(address, 3, 999).to_validated()).await.unwrap();
    let mut guard = mempool.inner.write().await;
    assert_eq!(guard.get_account_nonce(&contract_address).unwrap().0, Felt::THREE);
    assert_eq!(guard.pop_next_ready().unwrap().hash, Felt::from(999u64));
    guard.check_invariants();
}

#[tokio::test]
async fn admission_resolves_nonce_after_waiting_for_mempool_lock() {
    use futures::FutureExt;

    let backend = backend_with_genesis().await;
    let mempool = Mempool::new(backend.clone(), MempoolConfig::default());
    let address = Felt::from(123u64);
    mempool.accept_tx(executed(address, 0, 100).to_validated()).await.unwrap();
    let mut guard = mempool.inner.write().await;
    let admission = mempool.accept_tx(executed(address, 3, 999).to_validated());
    tokio::pin!(admission);
    assert!(admission.as_mut().now_or_never().is_none());

    assert_eq!(guard.pop_next_ready().unwrap().hash, Felt::from(100u64));
    for n in 1..=3 {
        append_block(&backend, n, vec![executed(address, n - 1, n)]);
    }
    drop(guard);
    admission.await.unwrap();
    let mut guard = mempool.inner.write().await;
    assert_eq!(guard.pop_next_ready().unwrap().hash, Felt::from(999u64));
    guard.check_invariants();
}

#[tokio::test]
async fn admission_falls_back_to_confirmed_nonce_below_empty_suffix() {
    let backend = backend_with_genesis().await;
    let address = Felt::from(123u64);
    append_block(&backend, 1, vec![executed(address, 0, 101)]);
    backend
        .write_access()
        .close_preconfirmed(
            true,
            1,
            StateDiff {
                nonces: vec![NonceUpdate { contract_address: address, nonce: Felt::ONE }],
                ..Default::default()
            },
        )
        .unwrap();
    append_block(&backend, 2, vec![]);
    append_block(&backend, 3, vec![]);
    let mempool = Mempool::new(backend, MempoolConfig::default());
    mempool.accept_tx(executed(address, 1, 999).to_validated()).await.unwrap();
    let mut guard = mempool.inner.write().await;
    assert_eq!(guard.pop_next_ready().unwrap().hash, Felt::from(999u64));
    guard.check_invariants();
}

#[tokio::test]
async fn older_confirmations_preserve_runahead_nonces_and_executed_statuses() {
    let backend = backend_with_genesis().await;
    let mempool = Mempool::new(backend.clone(), MempoolConfig::default());
    let address = Felt::from(123u64);
    let mut state = ChainWatcherState::new(Some(0));
    let mut subscription = backend.subscribe_internal_heads(mc_db::subscription::SubscribeNewBlocksTag::Preconfirmed);
    subscription.set_start_from(1);
    mempool.accept_tx(executed(address, 3, 999).to_validated()).await.unwrap();
    for n in 1..=3 {
        append_block(&backend, n, vec![executed(address, n - 1, n)]);
    }
    let effects = observe(&mempool, &mut state, subscription.next_block_view().await);
    mempool.apply_nonce_updates(effects.nonce_updates, effects.nonce_update_mode).await.unwrap();

    for n in 1..=3 {
        backend
            .write_access()
            .close_preconfirmed(
                true,
                n,
                StateDiff {
                    nonces: vec![NonceUpdate { contract_address: address, nonce: Felt::from(n) }],
                    ..Default::default()
                },
            )
            .unwrap();
        let effects = loop {
            let view = subscription.next_block_view().await;
            let is_confirmation = view.as_confirmed().is_some();
            let effects = observe(&mempool, &mut state, view);
            if is_confirmation {
                break effects;
            }
            assert!(effects.potentially_removed.is_empty(), "an unchanged runahead notification must be harmless");
        };
        assert_eq!(effects.nonce_updates[&address], Felt::from(3u64));
        assert!(effects.potentially_removed.is_empty(), "confirmation must not requeue canonical execution");
        assert_eq!(effects.confirmed_tx_hashes, vec![Felt::from(n)]);
        assert!(!mempool.preconfirmed_transactions_statuses.contains_key(&Felt::from(n)));
        for pending in (n + 1)..=3 {
            assert!(matches!(
                mempool.preconfirmed_transactions_statuses.get(&Felt::from(pending)).as_deref(),
                Some(PreConfirmationStatus::Executed { .. })
            ));
        }
        assert_eq!(state.preconfirmed.len(), (3 - n) as usize);
        mempool.apply_nonce_updates(effects.nonce_updates, effects.nonce_update_mode).await.unwrap();
        let guard = mempool.inner.read().await;
        assert_eq!(guard.get_account_nonce(&address.try_into().unwrap()).unwrap().0, Felt::from(3u64));
        guard.check_invariants();
    }
}

#[tokio::test]
async fn coalesced_frontier_loads_intermediate_blocks_and_accounts() {
    let backend = backend_with_genesis().await;
    let mempool = Mempool::new(backend.clone(), MempoolConfig::default());
    let mut state = ChainWatcherState::new(Some(0));
    append_block(&backend, 1, vec![executed(Felt::ONE, 0, 101)]);
    observe(&mempool, &mut state, backend.block_view_on_preconfirmed(1).unwrap().into());
    append_block(&backend, 2, vec![executed(Felt::TWO, 0, 102)]);
    append_block(&backend, 3, vec![executed(Felt::THREE, 0, 103)]);
    let effects = observe(&mempool, &mut state, backend.block_view_on_preconfirmed(3).unwrap().into());
    assert!(effects.potentially_removed.is_empty());
    assert_eq!(effects.nonce_updates[&Felt::TWO], Felt::ONE);
    assert_eq!(state.preconfirmed.len(), 3);
    for hash in 101..=103 {
        assert!(matches!(
            mempool.preconfirmed_transactions_statuses.get(&Felt::from(hash)).as_deref(),
            Some(PreConfirmationStatus::Executed { .. })
        ));
    }
}

#[tokio::test]
async fn replacement_requeues_only_removed_suffix_and_restores_parent_nonce() {
    let backend = backend_with_genesis().await;
    let mempool = Mempool::new(backend.clone(), MempoolConfig::default());
    let address = Felt::from(123u64);
    let mut state = ChainWatcherState::new(Some(0));
    append_block(&backend, 1, vec![executed(address, 0, 101)]);
    append_block(&backend, 2, vec![executed(address, 1, 102)]);
    observe(&mempool, &mut state, backend.block_view_on_preconfirmed(2).unwrap().into());
    // Replace the backend suffix, retaining the transaction in block 1.
    backend.write_access().clear_preconfirmed().unwrap();
    append_block(&backend, 1, vec![executed(address, 0, 101)]);
    append_block(&backend, 2, vec![]);
    let effects = observe(&mempool, &mut state, backend.block_view_on_preconfirmed(2).unwrap().into());
    assert_eq!(effects.potentially_removed.keys().copied().collect::<Vec<_>>(), vec![Felt::from(102u64)]);
    assert_eq!(effects.nonce_updates[&address], Felt::ONE);
    assert!(mempool.preconfirmed_transactions_statuses.contains_key(&Felt::from(101u64)));
}

#[tokio::test]
async fn retired_candidate_is_not_requeued_when_executed_in_later_block() {
    let backend = backend_with_genesis().await;
    let mempool = Mempool::new(backend.clone(), MempoolConfig::default());
    let address = Felt::from(123u64);
    let tx = executed(address, 0, 101);
    let mut state = ChainWatcherState::new(Some(0));
    append_block(&backend, 1, vec![]);
    backend.write_access().append_to_preconfirmed(1, &[], [Arc::new(tx.to_validated())]).unwrap();
    observe(&mempool, &mut state, backend.block_view_on_preconfirmed(1).unwrap().into());
    append_block(&backend, 2, vec![tx]);
    observe(&mempool, &mut state, backend.block_view_on_preconfirmed(2).unwrap().into());
    backend.write_access().close_preconfirmed(true, 1, StateDiff::default()).unwrap();
    let effects = observe(&mempool, &mut state, backend.block_view_on_confirmed(1).unwrap().into());
    assert!(effects.potentially_removed.is_empty());
    assert!(matches!(
        mempool.preconfirmed_transactions_statuses.get(&Felt::from(101u64)).as_deref(),
        Some(PreConfirmationStatus::Executed { .. })
    ));
}

#[tokio::test]
async fn handle_preconfirmed_content_update_disables_reinsertion_and_sets_candidate_status() {
    let backend = backend_with_genesis().await;
    let mempool = Mempool::new(backend.clone(), MempoolConfig::default());

    let candidate = Arc::new(crate::tests::tx_account(Felt::from(0x1234u64)));
    let block = Arc::new(PreconfirmedBlock::new_with_content(
        PreconfirmedHeader { block_number: 0, ..Default::default() },
        vec![],
        vec![candidate.clone()],
    ));
    let mut preconfirmed_view = mc_db::MadaraPreconfirmedBlockView::new(backend, block);
    let mut effects = ChainWatcherBranchEffects::new();

    mempool
        .handle_preconfirmed_content_update(&mut preconfirmed_view, &mut effects)
        .expect("preconfirmed content update");

    assert!(!effects.put_back_into_mempool, "candidates branch should drop potentially removed txs");
    assert!(effects.potentially_removed.is_empty(), "candidate is still present in refreshed view");

    let status = mempool
        .preconfirmed_transactions_statuses
        .get(&candidate.hash)
        .map(|status| status.clone())
        .expect("candidate status must be tracked");
    assert!(matches!(status, PreConfirmationStatus::Candidate { transaction_index: 0, .. }));
}

#[tokio::test]
async fn delayed_nonce_updates_only_regress_on_explicit_replacement() {
    let backend = backend_with_genesis().await;
    let mempool = Mempool::new(backend, MempoolConfig::default());
    let address = Felt::from(123u64);
    mempool.accept_tx(executed(address, 5, 999).to_validated()).await.unwrap();
    mempool.apply_nonce_updates([(address, Felt::THREE)].into(), NonceUpdateMode::Advance).await.unwrap();
    mempool.apply_nonce_updates([(address, Felt::ONE)].into(), NonceUpdateMode::Advance).await.unwrap();
    assert_eq!(mempool.inner.read().await.get_account_nonce(&address.try_into().unwrap()).unwrap().0, Felt::THREE);

    mempool.apply_nonce_updates([(address, Felt::ONE)].into(), NonceUpdateMode::Replace).await.unwrap();
    let guard = mempool.inner.read().await;
    assert_eq!(guard.get_account_nonce(&address.try_into().unwrap()).unwrap().0, Felt::ONE);
    guard.check_invariants();
}

#[tokio::test]
async fn reconstructed_preconfirmed_content_does_not_trigger_nonce_rollback() {
    let backend = backend_with_genesis().await;
    let mempool = Mempool::new(backend.clone(), MempoolConfig::default());
    let mut state = ChainWatcherState::new(Some(0));
    let tx = executed(Felt::ONE, 0, 101);
    append_block(&backend, 1, vec![tx.clone()]);
    observe(&mempool, &mut state, backend.block_view_on_preconfirmed(1).unwrap().into());
    backend.write_access().clear_preconfirmed().unwrap();
    append_block(&backend, 1, vec![tx]);
    let effects = observe(&mempool, &mut state, backend.block_view_on_preconfirmed(1).unwrap().into());
    assert!(effects.nonce_update_mode == NonceUpdateMode::Advance);
    assert!(effects.potentially_removed.is_empty());
}
