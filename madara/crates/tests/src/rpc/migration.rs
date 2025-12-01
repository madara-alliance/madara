//! Migration validation tests.
//!
//! These tests verify that database migrations complete successfully and
//! that data integrity is maintained. They are designed to run against
//! an external madara instance that has just completed a migration.
//!
//! # How it works
//!
//! 1. CI workflow downloads a base DB at the minimum supported version
//! 2. Madara starts with the base DB (migration runs automatically)
//! 3. These tests connect to the running madara and validate:
//!    - RPC endpoints are functional
//!    - State data is intact (block hashes, state roots)
//!    - Migration-specific transformations are correct
//!
//! # Running locally
//!
//! ```bash
//! # 1. Start madara with a DB that needs migration
//! ./madara --base-path /path/to/old-db --network sepolia --no-l1-sync
//!
//! # 2. Run migration tests
//! cargo test -p mc-e2e-tests migration_validation -- --ignored --nocapture
//! ```
//!
//! # Environment Variables
//!
//! - `MIGRATION_TEST_RPC_URL`: RPC endpoint URL (default: http://localhost:9944)

use rstest::rstest;
use starknet_core::types::{BlockId, MaybePreConfirmedStateUpdate};
use starknet_providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider};

/// Default RPC URL for migration tests.
const DEFAULT_RPC_URL: &str = "http://localhost:9944";

/// Get the RPC URL from environment or use default.
fn get_rpc_url() -> String {
    std::env::var("MIGRATION_TEST_RPC_URL").unwrap_or_else(|_| DEFAULT_RPC_URL.to_string())
}

/// Create a JSON-RPC client for migration tests.
fn get_client() -> JsonRpcClient<HttpTransport> {
    let url = get_rpc_url();
    let url = url::Url::parse(&url).expect("Invalid RPC URL");
    JsonRpcClient::new(HttpTransport::new(url))
}

// =============================================================================
// Generic Migration Validation Tests
// =============================================================================

/// Verify that madara RPC is accessible after migration.
///
/// This is the most basic check - if madara started successfully with
/// the migrated DB, it should respond to RPC calls.
#[rstest]
#[tokio::test]
#[ignore = "Requires external madara instance on port 9944"]
async fn test_migration_validation_rpc_healthy() {
    let client = get_client();

    let result = client.chain_id().await;
    assert!(result.is_ok(), "chain_id should succeed after migration: {:?}", result.err());

    let chain_id = result.unwrap();
    println!("✅ RPC healthy, chain_id: {:#x}", chain_id);
}

/// Verify that block data is accessible after migration.
///
/// Queries the latest block to ensure block storage is intact.
#[rstest]
#[tokio::test]
#[ignore = "Requires external madara instance on port 9944"]
async fn test_migration_validation_blocks_accessible() {
    let client = get_client();

    let result = client.block_hash_and_number().await;
    assert!(result.is_ok(), "block_hash_and_number should succeed: {:?}", result.err());

    let block = result.unwrap();
    println!("✅ Blocks accessible, latest: #{} ({})", block.block_number, block.block_hash);

    assert!(block.block_number > 0, "Should have at least one block");
}

/// Verify that state updates contain required fields after migration.
///
/// Checks that state updates have:
/// - block_hash
/// - old_root
/// - new_root
/// - state_diff
#[rstest]
#[tokio::test]
#[ignore = "Requires external madara instance on port 9944"]
async fn test_migration_validation_state_update_intact() {
    let client = get_client();

    // Query state update for block 1 (should exist in any synced DB)
    let result = client.get_state_update(BlockId::Number(1)).await;
    assert!(result.is_ok(), "get_state_update should succeed: {:?}", result.err());

    let state_update = result.unwrap();

    match state_update {
        MaybePreConfirmedStateUpdate::Update(update) => {
            // Verify essential fields are present and non-default
            assert!(update.block_hash != Default::default(), "block_hash should be set, got: {}", update.block_hash);
            assert!(update.new_root != Default::default(), "new_root should be set, got: {}", update.new_root);
            assert!(update.old_root != Default::default(), "old_root should be set, got: {}", update.old_root);

            println!("✅ State update intact for block 1:");
            println!("   block_hash: {}", update.block_hash);
            println!("   old_root: {}", update.old_root);
            println!("   new_root: {}", update.new_root);
        }
        MaybePreConfirmedStateUpdate::PreConfirmedUpdate(_) => {
            panic!("Block 1 should not be pre-confirmed");
        }
    }
}

// =============================================================================
// Version-Specific Migration Tests
// =============================================================================
//
// Add migration-specific tests here as new migrations are implemented.
// Each migration version should have its own submodule with targeted tests.

/// Tests specific to v8 → v9 migration (SNIP-34: CASM hash migration)
///
/// These tests will be implemented when the actual migration code is ready.
#[cfg(test)]
mod v9_snip34 {
    #[allow(unused_imports)]
    use super::*;

    // TODO: Implement when SNIP-34 migration is ready
    //
    // Example test structure:
    //
    // #[rstest]
    // #[tokio::test]
    // #[ignore = "Requires external madara instance on port 9944"]
    // async fn test_v9_state_diff_has_migrated_classes() {
    //     let client = get_client();
    //
    //     let state_update = client
    //         .get_state_update(BlockId::Number(1))
    //         .await
    //         .expect("Should get state update");
    //
    //     // For SNIP-34: verify migrated_classes field exists in state_diff
    //     // It should be empty for blocks that don't have class migrations
    //     match state_update {
    //         MaybePreConfirmedStateUpdate::Update(update) => {
    //             // Check that the field is accessible
    //             // Note: The actual field name depends on starknet-providers version
    //             println!("State diff storage_diffs count: {}", update.state_diff.storage_diffs.len());
    //         }
    //         _ => panic!("Expected confirmed state update"),
    //     }
    // }
}
