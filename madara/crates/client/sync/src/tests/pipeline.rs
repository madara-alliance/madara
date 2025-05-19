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
use mc_db::{db_block_id::DbBlockId, MadaraBackend};
use mc_settlement_client::state_update::StateUpdate;
use mp_chain_config::ChainConfig;
use mp_state_update::DeclaredClassItem;
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
    ctx.gateway_mock.mock_block_pending(felt!("0x13"));

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
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::UpdatedPendingBlock);

    assert_eq!(ctx.backend.get_block_hash(&DbBlockId::Number(0)).unwrap().unwrap(), felt!("0x10"));
    assert_eq!(ctx.backend.get_block_hash(&DbBlockId::Number(1)).unwrap().unwrap(), felt!("0x11"));
    assert_eq!(ctx.backend.get_block_hash(&DbBlockId::Number(2)).unwrap().unwrap(), felt!("0x12"));
    assert_eq!(ctx.backend.get_block_hash(&DbBlockId::Number(3)).unwrap().unwrap(), felt!("0x13"));
    assert!(ctx.backend.has_pending_block().unwrap());
    assert_eq!(
        ctx.backend
            .get_block_info(&DbBlockId::Pending)
            .unwrap()
            .unwrap()
            .as_pending()
            .unwrap()
            .header
            .parent_block_hash,
        felt!("0x13")
    );

    // add more blocks :)
    // pipeline should follow

    latest_mock.delete();
    ctx.gateway_mock.mock_block(4, felt!("0x14"), felt!("0x13"));
    ctx.gateway_mock.mock_block(5, felt!("0x15"), felt!("0x14"));
    ctx.gateway_mock.mock_block(6, felt!("0x16"), felt!("0x15"));
    ctx.gateway_mock.mock_header_latest(6, felt!("0x16"));
    ctx.gateway_mock.mock_block_pending(felt!("0x16"));

    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::SyncingTo { target: 6 });
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Idle);
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

    assert_eq!(ctx.backend.get_block_hash(&DbBlockId::Number(0)).unwrap().unwrap(), felt!("0x10"));
    assert_eq!(ctx.backend.get_block_hash(&DbBlockId::Number(1)).unwrap().unwrap(), felt!("0x11"));
    assert!(!ctx.backend.has_pending_block().unwrap());

    // 2. Pending block appears
    // add a pending block, pipeline should pick it up.

    pending_block_mock.delete();
    let mut pending_block_mock = ctx.gateway_mock.mock_block_pending_with_ts(felt!("0x11"), 1000000000000);

    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::UpdatedPendingBlock);

    assert!(ctx.backend.has_pending_block().unwrap());
    assert_eq!(
        ctx.backend
            .get_block_info(&DbBlockId::Pending)
            .unwrap()
            .unwrap()
            .as_pending()
            .unwrap()
            .header
            .block_timestamp
            .0,
        1000000000000
    );

    // 3. Pending block changes, we should reflect the change

    pending_block_mock.delete();
    ctx.gateway_mock.mock_block_pending_with_ts(felt!("0x11"), 1999999999999);

    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::UpdatedPendingBlock);

    assert!(ctx.backend.has_pending_block().unwrap());
    assert_eq!(
        ctx.backend
            .get_block_info(&DbBlockId::Pending)
            .unwrap()
            .unwrap()
            .as_pending()
            .unwrap()
            .header
            .block_timestamp
            .0,
        1999999999999
    );
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
    ctx.gateway_mock.mock_block_pending(felt!("0x13"));

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

    assert_eq!(ctx.backend.get_block_hash(&DbBlockId::Number(0)).unwrap().unwrap(), felt!("0x10"));
    assert_eq!(ctx.backend.get_block_hash(&DbBlockId::Number(1)).unwrap(), None);
    assert!(!ctx.backend.has_pending_block().unwrap());

    l1_snd.send(Some(StateUpdate { block_hash: felt!("0x12"), block_number: 2, global_root: Felt::ZERO })).unwrap();
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::SyncingTo { target: 2 });
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Idle);

    assert_eq!(ctx.backend.get_block_hash(&DbBlockId::Number(0)).unwrap().unwrap(), felt!("0x10"));
    assert_eq!(ctx.backend.get_block_hash(&DbBlockId::Number(1)).unwrap().unwrap(), felt!("0x11"));
    assert_eq!(ctx.backend.get_block_hash(&DbBlockId::Number(2)).unwrap().unwrap(), felt!("0x12"));
    assert_eq!(ctx.backend.get_block_hash(&DbBlockId::Number(3)).unwrap(), None);
    assert!(!ctx.backend.has_pending_block().unwrap());
}

#[rstest]
#[tokio::test]
/// Pending block is disabled.
async fn test_no_pending(mut ctx: TestContext) {
    ctx.gateway_mock.mock_block(0, felt!("0x10"), felt!("0x0"));
    ctx.gateway_mock.mock_header_latest(0, felt!("0x10"));
    ctx.gateway_mock.mock_block_pending(felt!("0x10"));

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

    assert_eq!(ctx.backend.get_block_hash(&DbBlockId::Number(0)).unwrap().unwrap(), felt!("0x10"));
    assert!(!ctx.backend.has_pending_block().unwrap());
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
    ctx.gateway_mock.mock_block_pending(felt!("0x13"));

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
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::UpdatedPendingBlock);
    assert_eq!(ctx.service_state_recv.recv().await, None); // task ended

    assert_eq!(ctx.backend.get_block_hash(&DbBlockId::Number(0)).unwrap().unwrap(), felt!("0x10"));
    assert_eq!(ctx.backend.get_block_hash(&DbBlockId::Number(1)).unwrap().unwrap(), felt!("0x11"));
    assert_eq!(ctx.backend.get_block_hash(&DbBlockId::Number(2)).unwrap().unwrap(), felt!("0x12"));
    assert_eq!(ctx.backend.get_block_hash(&DbBlockId::Number(3)).unwrap().unwrap(), felt!("0x13"));
    assert!(ctx.backend.has_pending_block().unwrap());
    assert_eq!(
        ctx.backend
            .get_block_info(&DbBlockId::Pending)
            .unwrap()
            .unwrap()
            .as_pending()
            .unwrap()
            .header
            .parent_block_hash,
        felt!("0x13")
    );

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

    assert_eq!(ctx.backend.get_block_hash(&DbBlockId::Number(0)).unwrap().unwrap(), felt!("0x10"));
    assert_eq!(ctx.backend.get_block_hash(&DbBlockId::Number(1)).unwrap(), None);

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

    assert_eq!(ctx.backend.get_block_hash(&DbBlockId::Number(1)).unwrap().unwrap(), felt!("0x11"));
    assert_eq!(ctx.backend.get_block_hash(&DbBlockId::Number(2)).unwrap().unwrap(), felt!("0x12"));
    // third block should not be imported
    assert_eq!(ctx.backend.get_block_hash(&DbBlockId::Number(3)).unwrap(), None);
    assert!(!ctx.backend.has_pending_block().unwrap());

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

    assert_eq!(ctx.backend.get_block_hash(&DbBlockId::Number(0)).unwrap().unwrap(), felt!("0x10"));
    assert_eq!(ctx.backend.get_block_hash(&DbBlockId::Number(1)).unwrap().unwrap(), felt!("0x11"));
    assert_eq!(ctx.backend.get_block_hash(&DbBlockId::Number(2)).unwrap(), None);
    assert!(!ctx.backend.has_pending_block().unwrap());

    task.await.unwrap(); // task returned.

    service_ctx.cancelled().await // global should be cancelled.
}

#[rstest]
#[tokio::test]
/// Test that we import the class if it's declared in a pending block. For classes declared in closed blocks,
/// it is already tested in the realistic tests.
async fn test_pending_declared_class(mut ctx: TestContext) {
    ctx.gateway_mock.mock_block(0, felt!("0x10"), felt!("0x0"));
    let mut mock = ctx.gateway_mock.mock_block_pending_not_found();
    ctx.gateway_mock.mock_header_latest(0, felt!("0x10"));

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
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::SyncingTo { target: 0 });
    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::Idle);

    assert_eq!(ctx.backend.get_block_hash(&DbBlockId::Number(0)).unwrap(), Some(felt!("0x10")));
    assert_eq!(ctx.backend.get_block_hash(&DbBlockId::Number(1)).unwrap(), None);
    assert!(!ctx.backend.has_pending_block().unwrap());

    mock.delete();

    ctx.gateway_mock.mock_class(m_cairo_test_contracts::TEST_CONTRACT_SIERRA);
    ctx.gateway_mock.mock_block_pending_with_declared_class(felt!("0x10"), 4242);

    assert_eq!(
        ctx.backend
            .get_class_info(
                &DbBlockId::Pending,
                &felt!("0x40fe2533528521fc49a8ad8440f8a1780c50337a94d0fce43756015fa816a8a")
            )
            .unwrap(),
        None
    );

    assert_eq!(ctx.service_state_recv.recv().await.unwrap(), ServiceEvent::UpdatedPendingBlock);

    assert!(ctx.backend.has_pending_block().unwrap());

    assert_eq!(
        ctx.backend
            .get_block_info(&DbBlockId::Pending)
            .unwrap()
            .unwrap()
            .as_pending()
            .unwrap()
            .header
            .block_timestamp
            .0,
        4242
    );
    assert_eq!(
        ctx.backend.get_block_state_diff(&DbBlockId::Pending).unwrap().unwrap().declared_classes,
        vec![DeclaredClassItem {
            class_hash: felt!("0x40fe2533528521fc49a8ad8440f8a1780c50337a94d0fce43756015fa816a8a"),
            compiled_class_hash: felt!("0x7d24ab3a5277e064c65b37f2bd4b118050a9f1864bd3f74beeb3e84b2213692")
        }]
    );
    assert!(ctx
        .backend
        .get_class_info(
            &DbBlockId::Pending,
            &felt!("0x40fe2533528521fc49a8ad8440f8a1780c50337a94d0fce43756015fa816a8a")
        )
        .unwrap()
        .is_some());
}
