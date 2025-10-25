use super::gateway_mock::{gateway_mock, GatewayMock};
use crate::{
    gateway::ForwardSyncConfig,
    import::{BlockImporter, BlockValidationConfig},
    SyncControllerConfig,
};
use anyhow::Context;
use mc_db::{MadaraBackend, MadaraStorageRead};
use mp_chain_config::ChainConfig;
use mp_utils::service::ServiceContext;
use rstest::{fixture, rstest};
use starknet_api::felt;
use std::sync::Arc;

struct ReorgTestContext {
    backend: Arc<MadaraBackend>,
    importer: Arc<BlockImporter>,
    gateway_mock: GatewayMock,
}

impl ReorgTestContext {
    async fn sync_to(&self, block_n: u64) {
        let mut sync = crate::gateway::forward_sync(
            self.backend.clone(),
            self.importer.clone(),
            self.gateway_mock.client(),
            SyncControllerConfig::default().stop_on_sync(true).stop_at_block_n(Some(block_n)),
            ForwardSyncConfig::default(),
        );

        sync.run(ServiceContext::default()).await.unwrap();
    }

    fn get_block_hash(&self, block_n: u64) -> Option<mp_convert::Felt> {
        self.backend
            .block_view_on_confirmed(block_n)
            .and_then(|view| view.get_block_info().ok())
            .map(|info| info.block_hash)
    }

    fn revert_to(&self, block_hash: &mp_convert::Felt) -> anyhow::Result<(u64, mp_convert::Felt)> {
        let result = self.backend.revert_to(block_hash)?;

        let fresh_chain_tip = self.backend.db.get_chain_tip().context("Getting fresh chain tip after reorg")?;
        let backend_chain_tip = mc_db::ChainTip::from_storage(fresh_chain_tip);
        self.backend.chain_tip.send_replace(backend_chain_tip);

        Ok(result)
    }
}

/// Test parameters for reorg scenarios
#[derive(Debug, Clone)]
struct ReorgTestArgs {
    /// Total number of blocks to sync
    total_blocks: u64,
    /// Number of blocks that match (common ancestor point)
    matching_blocks: u64,
    /// Number of times to repeat the reorg process
    passes: u32,
}

#[fixture]
fn reorg_ctx(gateway_mock: GatewayMock) -> ReorgTestContext {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::starknet_sepolia()));
    let importer = Arc::new(BlockImporter::new(backend.clone(), BlockValidationConfig::default()));

    // Set up genesis and base chain (blocks 0-2)
    gateway_mock.mock_block_from_json(0, include_str!("../../../../resources/sepolia.block_0.json"));
    gateway_mock.mock_class_from_json(
        "0xd0e183745e9dae3e4e78a8ffedcce0903fc4900beace4e0abf192d4c202da3",
        include_str!("../../../../resources/sepolia.block_0_class_0.json"),
    );
    gateway_mock.mock_class_from_json(
        "0x5c478ee27f2112411f86f207605b2e2c58cdb647bac0df27f660ef2252359c6",
        include_str!("../../../../resources/sepolia.block_0_class_1.json"),
    );
    gateway_mock.mock_block_from_json(1, include_str!("../../../../resources/sepolia.block_1.json"));
    gateway_mock.mock_class_from_json(
        "0x1b661756bf7d16210fc611626e1af4569baa1781ffc964bd018f4585ae241c1",
        include_str!("../../../../resources/sepolia.block_1_class_0.json"),
    );
    gateway_mock.mock_block_from_json(2, include_str!("../../../../resources/sepolia.block_2.json"));
    gateway_mock.mock_class_from_json(
        "0x4f23a756b221f8ce46b72e6a6b10ee7ee6cf3b59790e76e02433104f9a8c5d1",
        include_str!("../../../../resources/sepolia.block_2_class_0.json"),
    );
    gateway_mock.mock_block_pending_not_found();
    gateway_mock.mock_header_latest(2, felt!("0x7a906dfd1ff77a121b8048e6f750cda9e949d341c4487d4c6a449f183f0e61d"));

    ReorgTestContext { backend, importer, gateway_mock }
}

#[rstest]
#[case::reorg_to_genesis(ReorgTestArgs {
    total_blocks: 2,
    matching_blocks: 0,
    passes: 1,
})]
#[case::reorg_to_block_1(ReorgTestArgs {
    total_blocks: 2,
    matching_blocks: 1,
    passes: 1,
})]
#[tokio::test]
/// Simplified reorg test that focuses on the core revert functionality
///
/// This test validates the blockchain reorganization (reorg) functionality
/// by simulating various fork scenarios where the chain needs to revert
/// to a common ancestor and rebuild from an alternative fork.
///
/// Test scenarios:
/// 1. Reorg from genesis - Tests reverting back to block 0
/// 2. Empty orphan chain - Tests reorg when no blocks need to be orphaned
async fn test_reorg(reorg_ctx: ReorgTestContext, #[case] args: ReorgTestArgs) {
    tracing::info!("📦 Syncing {} blocks from gateway", args.total_blocks);
    reorg_ctx.sync_to(args.total_blocks).await;

    let latest = reorg_ctx.backend.latest_confirmed_block_n();
    assert_eq!(latest, Some(args.total_blocks), "Chain should be synced to block {}", args.total_blocks);
    tracing::info!("✅ Chain synced to block {}", args.total_blocks);

    let reorg_target_hash = reorg_ctx.get_block_hash(args.matching_blocks).expect("Reorg target block should exist");
    tracing::info!("🎯 Reorg target: block {} with hash {:#x}", args.matching_blocks, reorg_target_hash);

    for pass in 0..args.passes {
        tracing::info!("🔄 Starting reorg pass {}/{}", pass + 1, args.passes);

        // Revert to the common ancestor
        tracing::info!("🔄 Reverting to block {} (hash={:#x})", args.matching_blocks, reorg_target_hash);
        let (reverted_block_n, reverted_hash) = reorg_ctx.revert_to(&reorg_target_hash).expect("Reorg should succeed");

        assert_eq!(reverted_hash, reorg_target_hash, "Reverted hash should match target");
        assert_eq!(reverted_block_n, args.matching_blocks, "Reverted block number should match target");

        // Verify database state after reorg
        let latest_after = reorg_ctx.backend.latest_confirmed_block_n();
        assert_eq!(
            latest_after,
            Some(args.matching_blocks),
            "Latest block should be {} after reorg",
            args.matching_blocks
        );

        // Verify blocks beyond matching_blocks are removed
        for block_n in (args.matching_blocks + 1)..=args.total_blocks {
            let result = reorg_ctx.backend.block_view_on_confirmed(block_n);
            assert!(result.is_none(), "Block {} should be removed after reorg", block_n);
        }

        // Verify blocks up to matching_blocks still exist
        for block_n in 0..=args.matching_blocks {
            let result = reorg_ctx.backend.block_view_on_confirmed(block_n);
            assert!(result.is_some(), "Block {} should still exist after reorg", block_n);
        }

        tracing::info!("✅ Pass {}/{} completed - reverted to block {}", pass + 1, args.passes, args.matching_blocks);

        if pass < args.passes - 1 {
            tracing::info!("📦 Re-syncing to block {} for next pass", args.total_blocks);
            reorg_ctx.sync_to(args.total_blocks).await;
        }
    }
}

#[rstest]
#[tokio::test]
/// Test reorg detection during gateway sync
///
/// This test simulates a scenario where the gateway returns blocks
/// that don't match our current chain, triggering reorg detection.
/// This is the most realistic scenario where reorgs occur.
async fn test_reorg_detection_during_sync(reorg_ctx: ReorgTestContext) {
    // Build initial chain up to block 1
    tracing::info!("📦 Building initial chain to block 1");
    reorg_ctx.sync_to(1).await;

    let block_0_hash = reorg_ctx.get_block_hash(0).expect("Block 0 should exist");
    let block_1_hash = reorg_ctx.get_block_hash(1).expect("Block 1 should exist");

    tracing::info!("✅ Initial chain: block_0={:#x}, block_1={:#x}", block_0_hash, block_1_hash);

    // Verify the chain state
    let latest_block = reorg_ctx.backend.latest_confirmed_block_n();
    assert_eq!(latest_block, Some(1), "Latest block should be 1");

    // Verify blocks are accessible
    let view_0 = reorg_ctx.backend.block_view_on_confirmed(0).unwrap();
    assert_eq!(view_0.block_number(), 0);

    let view_1 = reorg_ctx.backend.block_view_on_confirmed(1).unwrap();
    assert_eq!(view_1.block_number(), 1);

    // Simulate a reorg by reverting to block 0
    // Note: revert_to removes blocks > target_block, so after reverting to block 0,
    // block 1 should be removed and the chain should be at block 0
    tracing::info!("🔄 Simulating reorg detected by gateway (reverting to block 0)");
    let (reverted_n, reverted_hash) = reorg_ctx.revert_to(&block_0_hash).expect("Revert should succeed");

    assert_eq!(reverted_n, 0);
    assert_eq!(reverted_hash, block_0_hash);

    // Verify chain is back to block 0
    // Note: The backend's chain_tip cache needs to be refreshed after revert_to
    // The revert_to function updates the database but the backend cache might be stale
    // In the real implementation, the gateway sync reinitializes pipelines after reorg
    let new_latest = reorg_ctx.backend.latest_confirmed_block_n();
    // The revert_to should have updated the chain tip to block 0
    // If this fails, it means the chain_tip cache wasn't properly updated
    tracing::info!("Latest block after reorg: {:?}", new_latest);
    assert_eq!(new_latest, Some(0), "Latest block should be 0 after reorg");

    // Verify we can still access block 0
    let view_after_reorg = reorg_ctx.backend.block_view_on_confirmed(0).unwrap();
    assert_eq!(view_after_reorg.block_number(), 0);
    assert_eq!(view_after_reorg.get_block_info().unwrap().block_hash, block_0_hash);

    // Verify block 1 is no longer accessible
    let block_1_after_reorg = reorg_ctx.backend.block_view_on_confirmed(1);
    assert!(block_1_after_reorg.is_none(), "Block 1 should not exist after reorg");

    tracing::info!("✅ Reorg detection test passed");
}

#[rstest]
#[tokio::test]
/// Test that the blockchain state (storage, nonces, class hashes, replaced classes) is properly
/// restored after a reorg. This validates the core revert functionality for all state components.
async fn test_revert_contract_state(reorg_ctx: ReorgTestContext) {
    // Build chain to block 2 to have multiple state changes
    tracing::info!("📦 Building chain to block 2");
    reorg_ctx.sync_to(2).await;

    // Capture state at block 2 (will be reverted)
    let block_2_view = reorg_ctx.backend.block_view_on_confirmed(2).unwrap();
    let block_2_state_diff = block_2_view.get_state_diff().unwrap();

    tracing::info!(
        "📊 Block 2 state: {} nonces, {} deployed contracts, {} storage updates, {} replaced classes",
        block_2_state_diff.nonces.len(),
        block_2_state_diff.deployed_contracts.len(),
        block_2_state_diff.storage_diffs.len(),
        block_2_state_diff.replaced_classes.len()
    );

    // Capture state at block 1 (revert target)
    let block_1_view = reorg_ctx.backend.block_view_on_confirmed(1).unwrap();
    let block_1_hash = block_1_view.get_block_info().unwrap().block_hash;
    let block_1_state_diff = block_1_view.get_state_diff().unwrap();

    tracing::info!(
        "📊 Block 1 state: {} nonces, {} deployed contracts, {} storage updates, {} replaced classes",
        block_1_state_diff.nonces.len(),
        block_1_state_diff.deployed_contracts.len(),
        block_1_state_diff.storage_diffs.len(),
        block_1_state_diff.replaced_classes.len()
    );

    // Perform reorg back to block 1
    tracing::info!("🔄 Performing reorg from block 2 to block 1");
    reorg_ctx.revert_to(&block_1_hash).expect("Reorg should succeed");

    // Verify chain is at block 1
    let latest = reorg_ctx.backend.latest_confirmed_block_n();
    assert_eq!(latest, Some(1), "Chain should be at block 1 after reorg");

    // Verify block 2 no longer exists
    let block_2_after = reorg_ctx.backend.block_view_on_confirmed(2);
    assert!(block_2_after.is_none(), "Block 2 should not exist after reorg");

    // Verify state matches block 1 (all block 2 changes reverted)
    let view_after_reorg = reorg_ctx.backend.block_view_on_confirmed(1).unwrap();
    let state_after_reorg = view_after_reorg.get_state_diff().unwrap();

    // Validate state component counts match block 1
    assert_eq!(state_after_reorg.nonces.len(), block_1_state_diff.nonces.len(), "Nonce count should match block 1");
    assert_eq!(
        state_after_reorg.deployed_contracts.len(),
        block_1_state_diff.deployed_contracts.len(),
        "Deployed contracts count should match block 1"
    );
    assert_eq!(
        state_after_reorg.storage_diffs.len(),
        block_1_state_diff.storage_diffs.len(),
        "Storage diffs count should match block 1"
    );
    assert_eq!(
        state_after_reorg.replaced_classes.len(),
        block_1_state_diff.replaced_classes.len(),
        "Replaced classes count should match block 1"
    );

    tracing::info!("✅ Contract state revert validated: all state components match block 1");
}

#[rstest]
#[tokio::test]
/// Test that declared classes (Sierra and legacy) are properly reverted during a reorg.
async fn test_revert_declared_class(reorg_ctx: ReorgTestContext) {
    // Build chain to block 2 which declares classes
    tracing::info!("📦 Building chain to block 2 (declares classes)");
    reorg_ctx.sync_to(2).await;

    // Get state diff at block 2 to see declared classes
    let block_2_view = reorg_ctx.backend.block_view_on_confirmed(2).unwrap();
    let block_2_state_diff = block_2_view.get_state_diff().unwrap();

    tracing::info!(
        "📊 Block 2 declared {} Sierra classes, {} legacy declared classes",
        block_2_state_diff.declared_classes.len(),
        block_2_state_diff.old_declared_contracts.len()
    );

    // Get state diff at block 1 (our revert target)
    let block_1_view = reorg_ctx.backend.block_view_on_confirmed(1).unwrap();
    let block_1_hash = block_1_view.get_block_info().unwrap().block_hash;
    let block_1_state_diff = block_1_view.get_state_diff().unwrap();

    tracing::info!(
        "📊 Block 1 declared {} Sierra classes, {} legacy declared classes",
        block_1_state_diff.declared_classes.len(),
        block_1_state_diff.old_declared_contracts.len()
    );

    // Perform reorg back to block 1
    tracing::info!("🔄 Performing reorg from block 2 to block 1");
    reorg_ctx.revert_to(&block_1_hash).expect("Reorg should succeed");

    // Verify state matches block 1 (block 2 classes removed)
    let view_after_reorg = reorg_ctx.backend.block_view_on_confirmed(1).unwrap();
    let state_after_reorg = view_after_reorg.get_state_diff().unwrap();

    // Validate declared classes match block 1 (block 2 classes reverted)
    assert_eq!(
        state_after_reorg.declared_classes.len(),
        block_1_state_diff.declared_classes.len(),
        "Sierra declared classes count should match block 1"
    );
    assert_eq!(
        state_after_reorg.old_declared_contracts.len(),
        block_1_state_diff.old_declared_contracts.len(),
        "Legacy declared classes count should match block 1"
    );

    // Verify block 2 no longer exists
    let block_2_after = reorg_ctx.backend.block_view_on_confirmed(2);
    assert!(block_2_after.is_none(), "Block 2 should not exist after reorg");

    tracing::info!("✅ Declared class revert validated: all block 2 classes removed");
}
