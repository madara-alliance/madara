//! Mocks a gateway, and checks the behavior of the gateway sync in isolation.
//! Commitments, hashes etc. are not checked - they should be checked separately in other tests.

use super::gateway_mock::{gateway_mock, GatewayMock};
use crate::{
    gateway::ForwardSyncConfig,
    import::{BlockImporter, BlockValidationConfig},
    sync::ServiceEvent,
    util::ServiceStateSender,
    SyncControllerConfig,
};
use mc_db::{
    preconfirmed::PreconfirmedBlock, MadaraBackend, MadaraBackendConfig, MadaraStorageRead, MadaraStorageWrite,
};
use mc_settlement_client::state_update::StateUpdate;
use mp_block::{header::PreconfirmedHeader, FullBlockWithoutCommitments};
use mp_chain_config::ChainConfig;
use mp_utils::{service::ServiceContext, AbortOnDrop};
use rstest::{fixture, rstest};
use starknet_api::felt;
use starknet_core::types::Felt;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedReceiver;

struct TestContext {
    backend: Arc<MadaraBackend>,
    importer: Arc<BlockImporter>,
    service_state_sender: ServiceStateSender<ServiceEvent>,
    service_state_recv: UnboundedReceiver<ServiceEvent>,
    gateway_mock: GatewayMock,
}

async fn poll_preconfirmed(
    gateway_mock: &GatewayMock,
    importer: &Arc<BlockImporter>,
    backend: &Arc<MadaraBackend>,
    disable_reorg_preconfirmed: bool,
) -> bool {
    let mut sync = crate::gateway::blocks::gateway_preconfirmed_block_sync(
        gateway_mock.client(),
        importer.clone(),
        backend.clone(),
        disable_reorg_preconfirmed,
    );
    sync.run().await.unwrap().is_some()
}

fn preconfirmed_state(backend: &Arc<MadaraBackend>) -> (u64, usize, usize) {
    let mut block = backend.block_view_on_current_preconfirmed().unwrap();
    block.refresh_with_candidates();
    (block.header().block_timestamp.0, block.num_executed_transactions(), block.candidate_transactions().len())
}

fn preconfirmed_transaction_hashes(backend: &Arc<MadaraBackend>) -> (Vec<Felt>, Vec<Felt>) {
    let mut block = backend.block_view_on_current_preconfirmed().unwrap();
    block.refresh_with_candidates();
    let executed = block.get_executed_transactions(..).into_iter().map(|tx| *tx.receipt.transaction_hash()).collect();
    let candidates = block.candidate_transactions().iter().map(|tx| tx.hash).collect();
    (executed, candidates)
}

#[fixture]
fn ctx(gateway_mock: GatewayMock) -> TestContext {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
    let importer = Arc::new(BlockImporter::new(
        backend.clone(),
        BlockValidationConfig::default().all_verifications_disabled(true),
    ));

    let (service_state_sender, service_state_recv) = crate::util::service_state_channel();

    TestContext { backend, importer, service_state_sender, service_state_recv, gateway_mock }
}
#[rstest]
#[tokio::test]
/// The pipeline should follow the mock_header_latest.
async fn test_probed(mut ctx: TestContext) {
    ctx.gateway_mock.mock_block(0, felt!("0x10"), felt!("0x0"));
    ctx.gateway_mock.mock_block(1, felt!("0x11"), felt!("0x10"));
    ctx.gateway_mock.mock_block(2, felt!("0x12"), felt!("0x11"));
    ctx.gateway_mock.mock_block(3, felt!("0x13"), felt!("0x12"));
    let mut latest_mock = ctx.gateway_mock.mock_header_latest(3, felt!("0x13"));
    ctx.gateway_mock.mock_block_pending(4);

    let mut sync = crate::gateway::forward_sync(
        ctx.backend.clone(),
        ctx.importer,
        ctx.gateway_mock.client(),
        SyncControllerConfig::default().service_state_sender(ctx.service_state_sender),
        ForwardSyncConfig::default(),
    );

    let _task = AbortOnDrop::spawn(async move { sync.run(ServiceContext::default()).await.unwrap() });

    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Starting);
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Idle);
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::SyncingTo { target: 3 });
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Idle);
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::UpdatedPreconfirmedBlock);

    assert_eq!(ctx.backend.block_view_on_confirmed(0).unwrap().get_block_info().unwrap().block_hash, felt!("0x10"));
    assert_eq!(ctx.backend.block_view_on_confirmed(1).unwrap().get_block_info().unwrap().block_hash, felt!("0x11"));
    assert_eq!(ctx.backend.block_view_on_confirmed(2).unwrap().get_block_info().unwrap().block_hash, felt!("0x12"));
    assert_eq!(ctx.backend.block_view_on_confirmed(3).unwrap().get_block_info().unwrap().block_hash, felt!("0x13"));
    assert!(ctx.backend.has_preconfirmed_block());
    assert_eq!(ctx.backend.block_view_on_current_preconfirmed().unwrap().header().block_number, 4);

    // add more blocks :)
    // pipeline should follow

    latest_mock.delete();
    ctx.gateway_mock.mock_block(4, felt!("0x14"), felt!("0x13"));
    ctx.gateway_mock.mock_block(5, felt!("0x15"), felt!("0x14"));
    ctx.gateway_mock.mock_block(6, felt!("0x16"), felt!("0x15"));
    ctx.gateway_mock.mock_header_latest(6, felt!("0x16"));
    ctx.gateway_mock.mock_block_pending(7);

    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::SyncingTo { target: 6 });
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Idle);
}

#[rstest]
#[tokio::test]
async fn test_stop_sync_on_unsupported_starknet_version(mut ctx: TestContext) {
    let unsupported_version = "0.14.3".to_string();

    ctx.gateway_mock.mock_block(0, felt!("0x10"), felt!("0x0"));
    ctx.gateway_mock.mock_block(1, felt!("0x11"), felt!("0x10"));
    ctx.gateway_mock.mock_block_with_starknet_version(2, felt!("0x12"), felt!("0x11"), unsupported_version.clone());
    ctx.gateway_mock.mock_header_latest(2, felt!("0x12"));
    ctx.gateway_mock.mock_block_pending_not_found();

    let mut sync = crate::gateway::forward_sync(
        ctx.backend.clone(),
        ctx.importer,
        ctx.gateway_mock.client(),
        SyncControllerConfig::default().service_state_sender(ctx.service_state_sender),
        ForwardSyncConfig::default(),
    );

    let task = AbortOnDrop::spawn(async move { sync.run(ServiceContext::default()).await });

    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Starting);
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Idle);
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::SyncingTo { target: 2 });

    let err = task.await.expect_err("sync should stop on the first unsupported Starknet version");
    let err = format!("{err:#}");
    assert!(err.contains(&format!("Unsupported Starknet version {unsupported_version}")), "{err}");
    assert!(err.contains("Latest supported version is"), "{err}");
    assert!(err.contains("block 2"), "{err}");

    // The sync aborts as soon as the unsupported block is encountered. Because the block, class,
    // and state pipelines run independently, earlier blocks may still be in flight and not yet
    // sealed as confirmed when the error is returned.
    assert!(ctx.backend.latest_confirmed_block_n().map(|n| n < 2).unwrap_or(true));
    assert_eq!(ctx.backend.block_view_on_confirmed(2), None);
    assert!(!ctx.backend.has_preconfirmed_block());
}

#[rstest]
#[tokio::test]
async fn test_pending_block_update(mut ctx: TestContext) {
    // 1. No pending block.
    ctx.gateway_mock.mock_block(0, felt!("0x10"), felt!("0x0"));
    ctx.gateway_mock.mock_block(1, felt!("0x11"), felt!("0x10"));
    ctx.gateway_mock.mock_header_latest(1, felt!("0x13"));
    let mut pending_block_mock = ctx.gateway_mock.mock_block_pending_not_found();

    let mut sync = crate::gateway::forward_sync(
        ctx.backend.clone(),
        ctx.importer,
        ctx.gateway_mock.client(),
        SyncControllerConfig::default().service_state_sender(ctx.service_state_sender),
        ForwardSyncConfig::default(),
    );

    let _task = AbortOnDrop::spawn(async move { sync.run(ServiceContext::default()).await.unwrap() });

    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Starting);
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Idle);
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::SyncingTo { target: 1 });
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Idle);

    assert_eq!(ctx.backend.block_view_on_confirmed(0).unwrap().get_block_info().unwrap().block_hash, felt!("0x10"));
    assert_eq!(ctx.backend.block_view_on_confirmed(1).unwrap().get_block_info().unwrap().block_hash, felt!("0x11"));
    assert!(!ctx.backend.has_preconfirmed_block());

    // 2. Pending block appears
    // add a pending block, pipeline should pick it up.

    pending_block_mock.delete();
    let mut pending_block_mock = ctx.gateway_mock.mock_block_pending_with_ts(2, 1000000000000);

    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::UpdatedPreconfirmedBlock);

    assert!(ctx.backend.has_preconfirmed_block());
    assert_eq!(ctx.backend.block_view_on_current_preconfirmed().unwrap().header().block_timestamp.0, 1000000000000);

    // 3. Pending block changes, we should reflect the change

    pending_block_mock.delete();
    ctx.gateway_mock.mock_block_pending_with_ts(2, 1999999999999);

    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::UpdatedPreconfirmedBlock);

    assert!(ctx.backend.has_preconfirmed_block());
    assert_eq!(ctx.backend.block_view_on_current_preconfirmed().unwrap().header().block_timestamp.0, 1999999999999);
}

/// Runs the complete sync controller until the mock gateway tip is reached.
async fn sync_to_tip(
    backend: &Arc<MadaraBackend>,
    gateway: &GatewayMock,
    no_pending_block: bool,
    disable_reorg_preconfirmed: bool,
) {
    let importer = Arc::new(BlockImporter::new(
        backend.clone(),
        BlockValidationConfig::default().all_verifications_disabled(true),
    ));
    let mut sync = crate::gateway::forward_sync(
        backend.clone(),
        importer,
        gateway.client(),
        SyncControllerConfig::default().stop_on_sync(true).no_pending_block(no_pending_block),
        ForwardSyncConfig::default().disable_reorg_preconfirmed(disable_reorg_preconfirmed),
    );
    tokio::time::timeout(std::time::Duration::from_secs(10), sync.run(ServiceContext::default()))
        .await
        .expect("sync should finish without retrying a rejected replacement forever")
        .unwrap();
}

#[rstest]
#[tokio::test]
async fn full_node_restart_discards_inherited_execution_suffix(
    gateway_mock: GatewayMock,
    #[values(true, false)] save_preconfirmed: bool,
    #[values(true, false)] disable_reorg_preconfirmed: bool,
) {
    let directory = tempfile::tempdir().unwrap();
    let open = |save_preconfirmed| {
        MadaraBackend::open_rocksdb(
            directory.path(),
            Arc::new(ChainConfig::madara_test()),
            MadaraBackendConfig { save_preconfirmed, ..Default::default() },
            Default::default(),
            Default::default(),
        )
        .unwrap()
    };
    let backend = open(true);
    // Use a genuinely computed confirmed root: recovery verifies it even though gateway mocks
    // elsewhere in this module intentionally bypass commitment verification.
    backend
        .write_access()
        .add_full_block_with_classes(
            &FullBlockWithoutCommitments {
                header: PreconfirmedHeader { block_number: 0, ..Default::default() },
                state_diff: Default::default(),
                transactions: vec![],
                events: vec![],
            },
            &[],
            false,
        )
        .unwrap();
    let genesis_hash = backend.block_view_on_confirmed(0).unwrap().get_block_info().unwrap().block_hash;
    let mut latest = gateway_mock.mock_header_latest(0, genesis_hash);

    // Persist a sequencer-style suffix before reopening the same database as a full node.
    for block_number in 1..=2 {
        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number, ..Default::default() }))
            .unwrap();
    }
    let abandoned_diff = mp_state_update::StateDiff {
        deployed_contracts: vec![mp_state_update::DeployedContractItem {
            address: felt!("0x123"),
            class_hash: felt!("0x456"),
        }],
        ..Default::default()
    };
    backend
        .write_access()
        .apply_to_global_trie(1, [&abandoned_diff], backend.chain_config().latest_protocol_version)
        .unwrap();
    // A finalizer may have persisted block parts without publishing confirmation.
    backend
        .write_access()
        .write_preconfirmed_with_precomputed_root(
            false,
            1,
            abandoned_diff,
            backend.db.get_state_root_hash().unwrap(),
            Default::default(),
        )
        .unwrap();
    assert!(backend.db.get_block_info(1).unwrap().is_some());
    assert_eq!(backend.db.get_contract_class_hash_at(1, &felt!("0x123")).unwrap(), Some(felt!("0x456")));
    backend.write_latest_applied_trie_update(&Some(1)).unwrap();
    backend.db.flush().unwrap();
    drop(backend);

    let backend = open(save_preconfirmed);
    assert_eq!(backend.chain_head_state().external_preconfirmed_tip, Some(1));
    assert_eq!(backend.chain_head_state().internal_preconfirmed_tip, Some(2));
    assert_ne!(backend.db.get_state_root_hash().unwrap(), Felt::ZERO);
    // Backend initialization removes partial block parts; sync startup must also reconcile
    // the independently materialized trie and discard the dependent preconfirmed suffix.
    assert!(backend.db.get_block_info(1).unwrap().is_none());
    assert_eq!(backend.db.get_contract_class_hash_at(1, &felt!("0x123")).unwrap(), None);
    let hashes = [felt!("0x99"), felt!("0x100")];
    gateway_mock.mock_block_pending_with_hashes(1, 12345, &hashes, 1);
    sync_to_tip(&backend, &gateway_mock, false, disable_reorg_preconfirmed).await;

    assert_eq!(preconfirmed_transaction_hashes(&backend), (vec![hashes[0]], vec![hashes[1]]));
    assert_eq!(backend.chain_head_state().internal_preconfirmed_tip, Some(1));
    assert_eq!(backend.latest_confirmed_block_n(), Some(0));
    assert_eq!(backend.block_view_on_confirmed(0).unwrap().get_block_info().unwrap().block_hash, genesis_hash);
    assert!(backend.block_view_on_preconfirmed(2).is_none());
    assert!(backend.db.get_preconfirmed_block_data(2).unwrap().is_none());
    assert_eq!(backend.db.get_state_root_hash().unwrap(), Felt::ZERO);
    assert_eq!(backend.get_latest_applied_trie_update().unwrap(), Some(0));
    backend.db.flush().unwrap();
    drop(backend);

    // A second restart must not resurrect the abandoned suffix, even if saving was disabled.
    let backend = open(save_preconfirmed);
    assert!(backend.block_view_on_preconfirmed(2).is_none());
    assert_eq!(backend.chain_head_state().internal_preconfirmed_tip, save_preconfirmed.then_some(1));
    latest.delete();
    gateway_mock.mock_header_latest(1, felt!("0x11"));
    gateway_mock.mock_block(1, felt!("0x11"), genesis_hash);
    sync_to_tip(&backend, &gateway_mock, true, disable_reorg_preconfirmed).await;
    assert_eq!(backend.latest_confirmed_block_n(), Some(1));
    assert_eq!(backend.block_view_on_confirmed(1).unwrap().get_block_info().unwrap().block_hash, felt!("0x11"));
    assert_eq!(backend.db.get_contract_class_hash_at(1, &felt!("0x123")).unwrap(), None);
}

#[rstest]
#[tokio::test]
async fn full_node_restart_reconciles_confirmed_trie_without_execution_suffix(
    gateway_mock: GatewayMock,
    #[values(false, true)] keep_preconfirmed: bool,
) {
    let directory = tempfile::tempdir().unwrap();
    let open = || {
        MadaraBackend::open_rocksdb(
            directory.path(),
            Arc::new(ChainConfig::madara_test()),
            MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
            Default::default(),
            Default::default(),
        )
        .unwrap()
    };
    let backend = open();
    backend
        .write_access()
        .add_full_block_with_classes(
            &FullBlockWithoutCommitments {
                header: PreconfirmedHeader { block_number: 0, ..Default::default() },
                state_diff: Default::default(),
                transactions: vec![],
                events: vec![],
            },
            &[],
            false,
        )
        .unwrap();
    backend.reconcile_confirmed_parallel_merkle_state("test_genesis").unwrap();
    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 1, ..Default::default() }))
        .unwrap();
    let diff = mp_state_update::StateDiff {
        deployed_contracts: vec![mp_state_update::DeployedContractItem {
            address: felt!("0x123"),
            class_hash: felt!("0x456"),
        }],
        ..Default::default()
    };
    let computed = backend
        .db
        .compute_root_from_latest_snapshot(1, &diff, backend.chain_config().latest_protocol_version, false)
        .unwrap();
    backend
        .write_access()
        .write_preconfirmed_with_precomputed_root(false, 1, diff, computed.state_root, computed.timings)
        .unwrap();
    backend.write_access().new_confirmed_block(1).unwrap();
    if keep_preconfirmed {
        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 2, ..Default::default() }))
            .unwrap();
    }
    backend.db.flush().unwrap();
    drop(backend);

    let backend = open();
    let head = backend.chain_head_state();
    let confirmed = backend.db.get_block_info(1).unwrap().unwrap();
    assert_eq!(head.confirmed_tip, Some(1));
    assert_eq!(head.external_preconfirmed_tip, keep_preconfirmed.then_some(2));
    assert_eq!(head.internal_preconfirmed_tip, head.external_preconfirmed_tip);
    assert_ne!(backend.db.get_state_root_hash().unwrap(), confirmed.header.global_state_root);
    let importer = Arc::new(BlockImporter::new(backend.clone(), BlockValidationConfig::default()));
    let mut sync = crate::gateway::forward_sync(
        backend.clone(),
        importer.clone(),
        gateway_mock.client(),
        SyncControllerConfig::default().no_pending_block(true),
        ForwardSyncConfig::default().disable_reorg_preconfirmed(true),
    );
    // Exercise startup without allowing upstream polling to repair the fixture.
    let ctx = ServiceContext::default();
    ctx.cancel_global();
    sync.run(ctx).await.unwrap();
    assert_eq!(backend.chain_head_state(), head);
    assert_eq!(backend.db.get_state_root_hash().unwrap(), confirmed.header.global_state_root);
    assert_eq!(backend.get_latest_applied_trie_update().unwrap(), Some(1));

    // Startup must flush the repair so another restart retains both the trie and head policy.
    drop(sync);
    drop(importer);
    drop(backend);
    let backend = open();
    assert_eq!(backend.chain_head_state(), head);
    assert_eq!(backend.db.get_state_root_hash().unwrap(), confirmed.header.global_state_root);
    assert_eq!(backend.get_latest_applied_trie_update().unwrap(), Some(1));
    let importer = Arc::new(BlockImporter::new(backend.clone(), BlockValidationConfig::default()));

    // An empty upstream block must preserve block 1's root; verification stays enabled.
    let mut header = confirmed.header;
    header.block_number = 2;
    backend
        .write_access()
        .write_header(mp_block::BlockHeaderWithSignatures {
            header,
            block_hash: felt!("0x999"),
            consensus_signatures: vec![],
        })
        .unwrap();
    importer.run_in_rayon_pool_global(|ctx| ctx.apply_to_global_trie(2..3, vec![Default::default()])).await.unwrap();
}

#[rstest]
#[tokio::test]
async fn full_node_startup_preserves_independent_trie_progress(ctx: TestContext) {
    ctx.backend
        .write_access()
        .add_full_block_with_classes(
            &FullBlockWithoutCommitments {
                header: PreconfirmedHeader { block_number: 0, ..Default::default() },
                state_diff: Default::default(),
                transactions: vec![],
                events: vec![],
            },
            &[],
            false,
        )
        .unwrap();
    let diff = mp_state_update::StateDiff {
        deployed_contracts: vec![mp_state_update::DeployedContractItem {
            address: felt!("0x123"),
            class_hash: felt!("0x456"),
        }],
        ..Default::default()
    };
    ctx.backend
        .write_access()
        .apply_to_global_trie(1, [&diff], ctx.backend.chain_config().latest_protocol_version)
        .unwrap();
    ctx.backend.write_latest_applied_trie_update(&Some(1)).unwrap();
    let root = ctx.backend.db.get_state_root_hash().unwrap();
    assert_ne!(root, Felt::ZERO);
    let head = ctx.backend.chain_head_state();
    assert_eq!(head.confirmed_tip, Some(0));
    assert_eq!(head.internal_preconfirmed_tip, None);

    let mut sync = crate::gateway::forward_sync(
        ctx.backend.clone(),
        ctx.importer,
        ctx.gateway_mock.client(),
        SyncControllerConfig::default(),
        ForwardSyncConfig::default(),
    );
    let service = ServiceContext::default();
    service.cancel_global();
    sync.run(service).await.unwrap();
    assert_eq!(ctx.backend.chain_head_state(), head);
    assert_eq!(ctx.backend.db.get_state_root_hash().unwrap(), root);
    assert_eq!(ctx.backend.get_latest_applied_trie_update().unwrap(), Some(1));
}

#[rstest]
#[tokio::test]
async fn full_node_startup_discards_execution_before_genesis(gateway_mock: GatewayMock) {
    let backend = MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_test()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    );
    for block_number in 0..=1 {
        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number, ..Default::default() }))
            .unwrap();
    }
    backend.write_latest_applied_trie_update(&Some(0)).unwrap();
    sync_to_tip(&backend, &gateway_mock, true, false).await;
    assert_eq!(backend.chain_head_state(), Default::default());
    assert_eq!(backend.get_latest_applied_trie_update().unwrap(), None);
    for block_number in 0..=1 {
        assert!(backend.db.get_preconfirmed_block_data(block_number).unwrap().is_none());
    }
    backend.refresh_head_projection_from_db().unwrap();
    assert_eq!(backend.chain_head_state(), Default::default());
}

#[rstest]
#[tokio::test]
async fn full_node_startup_preserves_single_preconfirmed_when_reorgs_disabled(ctx: TestContext) {
    let mut pending = ctx.gateway_mock.mock_block_pending_with_hashes(0, 12345, &[felt!("0x1")], 1);
    assert!(poll_preconfirmed(&ctx.gateway_mock, &ctx.importer, &ctx.backend, false).await);
    pending.delete();
    ctx.gateway_mock.mock_block_pending_with_hashes(0, 54321, &[felt!("0x2")], 1);
    // There are no confirmed blocks yet, so only the preconfirmed poll can make progress.
    sync_to_tip(&ctx.backend, &ctx.gateway_mock, false, true).await;
    assert_eq!(preconfirmed_transaction_hashes(&ctx.backend), (vec![felt!("0x1")], vec![]));
    assert_eq!(preconfirmed_state(&ctx.backend), (12345, 1, 0));
}

#[rstest]
#[tokio::test]
async fn test_pending_block_reorg_disabled(mut ctx: TestContext) {
    ctx.gateway_mock.mock_block(0, felt!("0x10"), felt!("0x0"));
    ctx.gateway_mock.mock_block(1, felt!("0x11"), felt!("0x10"));
    let mut latest_mock = ctx.gateway_mock.mock_header_latest(1, felt!("0x11"));
    let mut pending_block_mock = ctx.gateway_mock.mock_block_pending_with_ts_and_counts(2, 1000000000000, 2, 1);

    let mut sync = crate::gateway::forward_sync(
        ctx.backend.clone(),
        ctx.importer.clone(),
        ctx.gateway_mock.client(),
        SyncControllerConfig::default().service_state_sender(ctx.service_state_sender),
        ForwardSyncConfig::default().disable_reorg_preconfirmed(true),
    );

    let task = AbortOnDrop::spawn(async move { sync.run(ServiceContext::default()).await.unwrap() });

    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Starting);
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Idle);
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::SyncingTo { target: 1 });
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Idle);
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::UpdatedPreconfirmedBlock);

    assert_eq!(preconfirmed_state(&ctx.backend), (1000000000000, 1, 1));
    drop(task);

    pending_block_mock.delete();
    let mut grown_block_mock = ctx.gateway_mock.mock_block_pending_with_ts_and_counts(2, 1000000000000, 3, 2);
    assert!(poll_preconfirmed(&ctx.gateway_mock, &ctx.importer, &ctx.backend, true).await);
    assert_eq!(preconfirmed_state(&ctx.backend), (1000000000000, 2, 1));

    grown_block_mock.delete();
    let hashes =
        [felt!("0x6a5a493cf33919e58aa4c75777bffdef97c0e39cac968896d7bee8cc67905a1"), felt!("0x2"), felt!("0x67")];
    let mut candidate_replaced_block_mock =
        ctx.gateway_mock.mock_block_pending_with_hashes(2, 1000000000000, &hashes, 2);
    assert!(!poll_preconfirmed(&ctx.gateway_mock, &ctx.importer, &ctx.backend, true).await);
    assert_eq!(preconfirmed_state(&ctx.backend), (1000000000000, 2, 1));

    candidate_replaced_block_mock.delete();
    let mut shrunk_block_mock = ctx.gateway_mock.mock_block_pending_with_ts_and_count(2, 1000000000000, 2);
    assert!(!poll_preconfirmed(&ctx.gateway_mock, &ctx.importer, &ctx.backend, true).await);
    assert_eq!(preconfirmed_state(&ctx.backend), (1000000000000, 2, 1));

    shrunk_block_mock.delete();
    let hashes = [felt!("0x65"), felt!("0x66"), felt!("0x67")];
    let mut replaced_block_mock = ctx.gateway_mock.mock_block_pending_with_hashes(2, 1000000000000, &hashes, 2);
    assert!(!poll_preconfirmed(&ctx.gateway_mock, &ctx.importer, &ctx.backend, true).await);
    assert_eq!(preconfirmed_state(&ctx.backend), (1000000000000, 2, 1));

    replaced_block_mock.delete();
    let mut empty_block_mock = ctx.gateway_mock.mock_block_pending_with_ts_and_count(2, 1000000000000, 0);
    assert!(!poll_preconfirmed(&ctx.gateway_mock, &ctx.importer, &ctx.backend, true).await);
    assert_eq!(preconfirmed_state(&ctx.backend), (1000000000000, 2, 1));

    empty_block_mock.delete();
    let mut header_replaced_block_mock = ctx.gateway_mock.mock_block_pending_with_ts_and_counts(2, 1999999999999, 3, 2);
    assert!(!poll_preconfirmed(&ctx.gateway_mock, &ctx.importer, &ctx.backend, true).await);
    assert_eq!(preconfirmed_state(&ctx.backend), (1000000000000, 2, 1));

    header_replaced_block_mock.delete();
    latest_mock.delete();
    ctx.gateway_mock.mock_block(2, felt!("0x12"), felt!("0x11"));
    ctx.gateway_mock.mock_header_latest(2, felt!("0x12"));
    ctx.gateway_mock.mock_block_pending_with_ts(3, 2000000000000);

    let mut sync = crate::gateway::forward_sync(
        ctx.backend.clone(),
        ctx.importer,
        ctx.gateway_mock.client(),
        SyncControllerConfig::default().stop_on_sync(true),
        ForwardSyncConfig::default().disable_reorg_preconfirmed(true),
    );
    sync.run(ServiceContext::default()).await.unwrap();

    assert_eq!(ctx.backend.latest_confirmed_block_n(), Some(2));
    assert_eq!(ctx.backend.block_view_on_current_preconfirmed().unwrap().header().block_number, 3);
}

#[rstest]
#[tokio::test]
async fn test_pending_block_reorg_allowed_by_default(ctx: TestContext) {
    let hashes = [felt!("0x1"), felt!("0x2")];
    let mut pending_block_mock = ctx.gateway_mock.mock_block_pending_with_hashes(0, 1000000000000, &hashes, 1);
    assert!(poll_preconfirmed(&ctx.gateway_mock, &ctx.importer, &ctx.backend, false).await);
    assert_eq!(preconfirmed_transaction_hashes(&ctx.backend), (vec![felt!("0x1")], vec![felt!("0x2")]));

    pending_block_mock.delete();
    let hashes = [felt!("0x1"), felt!("0x2"), felt!("0x3")];
    let mut grown_block_mock = ctx.gateway_mock.mock_block_pending_with_hashes(0, 1000000000000, &hashes, 2);
    assert!(poll_preconfirmed(&ctx.gateway_mock, &ctx.importer, &ctx.backend, false).await);
    assert_eq!(preconfirmed_transaction_hashes(&ctx.backend), (vec![felt!("0x1"), felt!("0x2")], vec![felt!("0x3")]));

    grown_block_mock.delete();
    let hashes = [felt!("0x1"), felt!("0x2"), felt!("0x4")];
    ctx.gateway_mock.mock_block_pending_with_hashes(0, 1000000000000, &hashes, 2);
    assert!(poll_preconfirmed(&ctx.gateway_mock, &ctx.importer, &ctx.backend, false).await);
    assert_eq!(preconfirmed_transaction_hashes(&ctx.backend), (vec![felt!("0x1"), felt!("0x2")], vec![felt!("0x4")]));
}

#[rstest]
#[tokio::test]
/// First, make the pipeline sync to block 0.
/// Then, send an l1 head update, the pipeline should follow.
async fn test_follows_l1(mut ctx: TestContext) {
    ctx.gateway_mock.mock_block(0, felt!("0x10"), felt!("0x0"));
    ctx.gateway_mock.mock_block(1, felt!("0x11"), felt!("0x10"));
    ctx.gateway_mock.mock_block(2, felt!("0x12"), felt!("0x11"));
    ctx.gateway_mock.mock_block(3, felt!("0x13"), felt!("0x12"));
    ctx.gateway_mock.mock_header_latest(0, felt!("0x10"));
    ctx.gateway_mock.mock_block_pending(4);

    let (l1_snd, l1_recv) = tokio::sync::watch::channel(None);

    let mut sync = crate::gateway::forward_sync(
        ctx.backend.clone(),
        ctx.importer,
        ctx.gateway_mock.client(),
        SyncControllerConfig::default().service_state_sender(ctx.service_state_sender).l1_head_recv(l1_recv),
        ForwardSyncConfig::default(),
    );

    let _task = AbortOnDrop::spawn(async move { sync.run(ServiceContext::default()).await.unwrap() });

    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Starting);
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Idle);
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::SyncingTo { target: 0 });
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Idle);

    assert_eq!(ctx.backend.block_view_on_confirmed(0).unwrap().get_block_info().unwrap().block_hash, felt!("0x10"));
    assert_eq!(ctx.backend.block_view_on_confirmed(1), None);
    assert!(!ctx.backend.has_preconfirmed_block());

    l1_snd
        .send(Some(StateUpdate { block_hash: felt!("0x12"), block_number: Some(2), global_root: Felt::ZERO }))
        .unwrap();
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::SyncingTo { target: 2 });
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Idle);

    assert_eq!(ctx.backend.block_view_on_confirmed(0).unwrap().get_block_info().unwrap().block_hash, felt!("0x10"));
    assert_eq!(ctx.backend.block_view_on_confirmed(1).unwrap().get_block_info().unwrap().block_hash, felt!("0x11"));
    assert_eq!(ctx.backend.block_view_on_confirmed(2).unwrap().get_block_info().unwrap().block_hash, felt!("0x12"));
    assert_eq!(ctx.backend.block_view_on_confirmed(3), None);
    assert!(!ctx.backend.has_preconfirmed_block());
}

#[rstest]
#[tokio::test]
/// Pending block is disabled.
async fn test_no_pending(mut ctx: TestContext) {
    ctx.gateway_mock.mock_block(0, felt!("0x10"), felt!("0x0"));
    ctx.gateway_mock.mock_header_latest(0, felt!("0x10"));
    ctx.gateway_mock.mock_block_pending(1);

    let mut sync = crate::gateway::forward_sync(
        ctx.backend.clone(),
        ctx.importer,
        ctx.gateway_mock.client(),
        SyncControllerConfig::default().service_state_sender(ctx.service_state_sender).no_pending_block(true),
        ForwardSyncConfig::default(),
    );

    let _task = AbortOnDrop::spawn(async move { sync.run(ServiceContext::default()).await.unwrap() });

    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Starting);
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Idle);
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::SyncingTo { target: 0 });
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Idle);

    assert_eq!(ctx.backend.block_view_on_confirmed(0).unwrap().get_block_info().unwrap().block_hash, felt!("0x10"));
    assert!(!ctx.backend.has_preconfirmed_block());
}

#[rstest]
#[tokio::test]
/// The pipeline should stop once fully synced.
async fn test_stop_on_sync(mut ctx: TestContext) {
    ctx.gateway_mock.mock_class(m_cairo_test_contracts::TEST_CONTRACT_SIERRA);
    ctx.gateway_mock.mock_block(0, felt!("0x10"), felt!("0x0"));
    ctx.gateway_mock.mock_block(1, felt!("0x11"), felt!("0x10"));
    ctx.gateway_mock.mock_block(2, felt!("0x12"), felt!("0x11"));
    ctx.gateway_mock.mock_block(3, felt!("0x13"), felt!("0x12"));
    ctx.gateway_mock.mock_header_latest(3, felt!("0x13"));
    ctx.gateway_mock.mock_block_pending(4);

    let mut sync = crate::gateway::forward_sync(
        ctx.backend.clone(),
        ctx.importer,
        ctx.gateway_mock.client(),
        SyncControllerConfig::default().service_state_sender(ctx.service_state_sender).stop_on_sync(true),
        ForwardSyncConfig::default(),
    );

    let task = AbortOnDrop::spawn(async move { sync.run(ServiceContext::default()).await.unwrap() });

    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Starting);
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Idle);
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::SyncingTo { target: 3 });
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Idle);
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::UpdatedPreconfirmedBlock);
    assert_eq!(ctx.service_state_recv.recv().await, None); // task ended

    assert_eq!(ctx.backend.block_view_on_confirmed(0).unwrap().get_block_info().unwrap().block_hash, felt!("0x10"));
    assert_eq!(ctx.backend.block_view_on_confirmed(1).unwrap().get_block_info().unwrap().block_hash, felt!("0x11"));
    assert_eq!(ctx.backend.block_view_on_confirmed(2).unwrap().get_block_info().unwrap().block_hash, felt!("0x12"));
    assert_eq!(ctx.backend.block_view_on_confirmed(3).unwrap().get_block_info().unwrap().block_hash, felt!("0x13"));
    assert!(ctx.backend.has_preconfirmed_block());
    assert_eq!(ctx.backend.block_view_on_current_preconfirmed().unwrap().header().block_number, 4);

    task.await // task returned.
}

#[rstest]
#[tokio::test]
/// The pipeline should stop once at block_n.
async fn test_stop_at_block_n(mut ctx: TestContext) {
    ctx.gateway_mock.mock_class(m_cairo_test_contracts::TEST_CONTRACT_SIERRA);
    ctx.gateway_mock.mock_block(0, felt!("0x10"), felt!("0x0"));
    ctx.gateway_mock.mock_block_pending_not_found();
    let mut latest_mock = ctx.gateway_mock.mock_header_latest(0, felt!("0x10"));

    let mut sync = crate::gateway::forward_sync(
        ctx.backend.clone(),
        ctx.importer,
        ctx.gateway_mock.client(),
        SyncControllerConfig::default().service_state_sender(ctx.service_state_sender).stop_at_block_n(Some(2)),
        ForwardSyncConfig::default(),
    );

    let task = AbortOnDrop::spawn(async move { sync.run(ServiceContext::default()).await.unwrap() });

    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Starting);
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Idle);
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::SyncingTo { target: 0 });
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Idle);

    assert_eq!(ctx.backend.block_view_on_confirmed(0).unwrap().get_block_info().unwrap().block_hash, felt!("0x10"));
    assert_eq!(ctx.backend.block_view_on_confirmed(1), None);

    // task should not have ended yet, as we havent reached the stop condition (even though
    // there are no blocks to import yet)

    // add more blocks now

    latest_mock.delete();
    ctx.gateway_mock.mock_block(1, felt!("0x11"), felt!("0x10"));
    ctx.gateway_mock.mock_block(2, felt!("0x12"), felt!("0x11"));
    ctx.gateway_mock.mock_block(3, felt!("0x13"), felt!("0x12"));
    ctx.gateway_mock.mock_header_latest(3, felt!("0x13"));

    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::SyncingTo { target: 2 });
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Idle);
    assert_eq!(ctx.service_state_recv.recv().await, None); // task ended

    assert_eq!(ctx.backend.block_view_on_confirmed(1).unwrap().get_block_info().unwrap().block_hash, felt!("0x11"));
    assert_eq!(ctx.backend.block_view_on_confirmed(2).unwrap().get_block_info().unwrap().block_hash, felt!("0x12"));
    // third block should not be imported
    assert_eq!(ctx.backend.block_view_on_confirmed(3), None);
    assert!(!ctx.backend.has_preconfirmed_block());

    task.await // task returned.
}

#[rstest]
#[tokio::test]
/// The pipeline should stop once fully synced.
/// Unsure: should we also sync the pending block? it's debatable
async fn test_global_stop(mut ctx: TestContext) {
    ctx.gateway_mock.mock_class(m_cairo_test_contracts::TEST_CONTRACT_SIERRA);
    ctx.gateway_mock.mock_block(0, felt!("0x10"), felt!("0x0"));
    ctx.gateway_mock.mock_block(1, felt!("0x11"), felt!("0x10"));
    ctx.gateway_mock.mock_block(2, felt!("0x12"), felt!("0x11"));
    ctx.gateway_mock.mock_header_latest(2, felt!("0x13"));
    ctx.gateway_mock.mock_block_pending_not_found();

    let mut sync = crate::gateway::forward_sync(
        ctx.backend.clone(),
        ctx.importer,
        ctx.gateway_mock.client(),
        SyncControllerConfig::default()
            .service_state_sender(ctx.service_state_sender)
            .stop_on_sync(true)
            .global_stop_on_sync(true)
            .stop_at_block_n(Some(1)),
        ForwardSyncConfig::default(),
    );

    let mut service_ctx = ServiceContext::default();
    let service_ctx_ = service_ctx.clone();
    let task = AbortOnDrop::spawn(async move { sync.run(service_ctx_.child()).await });

    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Starting);
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Idle);
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::SyncingTo { target: 1 });
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Idle);
    assert_eq!(ctx.service_state_recv.recv().await, None); // task ended

    assert_eq!(ctx.backend.block_view_on_confirmed(0).unwrap().get_block_info().unwrap().block_hash, felt!("0x10"));
    assert_eq!(ctx.backend.block_view_on_confirmed(1).unwrap().get_block_info().unwrap().block_hash, felt!("0x11"));
    assert_eq!(ctx.backend.block_view_on_confirmed(2), None);
    assert!(!ctx.backend.has_preconfirmed_block());

    task.await.unwrap(); // task returned.

    service_ctx.cancelled().await // global should be cancelled.
}
