use crate::{
    global_spawn_rayon_task, BlockImportError, BlockImportResult, BlockValidationContext, PendingBlockImportResult,
    PreValidatedBlock, PreValidatedPendingBlock, ReorgResult, UnverifiedHeader, ValidatedCommitments,
};
use bonsai_trie::id::BasicId;
use itertools::Itertools;
use mc_db::{MadaraBackend, MadaraStorageError};
use mp_block::BlockTag;
use mp_block::{
    header::PendingHeader, BlockId, Header, MadaraBlockInfo, MadaraBlockInner, MadaraMaybePendingBlock,
    MadaraMaybePendingBlockInfo, MadaraPendingBlockInfo,
};
use mp_convert::{FeltHexDisplay, ToFelt};
use starknet_api::core::ChainId;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Poseidon, StarkHash};
use std::{borrow::Cow, sync::Arc};

mod classes;
mod contracts;

pub struct VerifyApply {
    pub(crate) backend: Arc<MadaraBackend>,
    // Only one thread at once can verify_apply. This is the update trie step cannot be parallelized over blocks, and in addition
    // our database does not support concurrent write access.
    mutex: tokio::sync::Mutex<()>,
}

impl VerifyApply {
    pub fn new(backend: Arc<MadaraBackend>) -> Self {
        Self { backend, mutex: Default::default() }
    }

    /// This function wraps the [`verify_apply_inner`] step, which runs on the rayon pool, in a tokio-friendly future.
    ///
    /// NOTE: we do not use [`crate::rayon::RayonPool`], but [`global_spawn_rayon_task`] - this is because that would allow for a deadlock if we were to share
    /// the semaphore with the [`crate::pre_validate`] task.
    /// This is fine because the [`VerifyApply::mutex`] ensures correct backpressure handling.
    pub async fn verify_apply(
        &self,
        block: PreValidatedBlock,
        validation: BlockValidationContext,
    ) -> Result<BlockImportResult, BlockImportError> {
        tracing::debug!("acquiring verify_apply exclusive");
        let _exclusive = self.mutex.lock().await;
        tracing::debug!("acquired verify_apply exclusive");

        let backend = Arc::clone(&self.backend);
        let res = global_spawn_rayon_task(move || verify_apply_inner(&backend, block, validation)).await;
        tracing::debug!("releasing verify_apply exclusive");
        res
    }

    /// See [`Self::verify_apply`].
    pub async fn verify_apply_pending(
        &self,
        block: PreValidatedPendingBlock,
        validation: BlockValidationContext,
    ) -> Result<PendingBlockImportResult, BlockImportError> {
        tracing::debug!("acquiring verify_apply exclusive (pending)");
        let _exclusive = self.mutex.lock().await;
        tracing::debug!("acquired verify_apply exclusive (pending)");

        let backend = Arc::clone(&self.backend);
        let res = global_spawn_rayon_task(move || verify_apply_pending_inner(&backend, block, validation)).await;
        tracing::debug!("releasing verify_apply exclusive (pending)");
        res
    }
}

/// This needs to be called sequentially, it will apply the state diff to the db, verify the state root and save the block.
/// This runs on the [`rayon`] threadpool however as it uses parallelism inside.
// TODO(perf): Investigate what we can overlap between block storage and trie updates
pub fn verify_apply_inner(
    backend: &MadaraBackend,
    block: PreValidatedBlock,
    validation: BlockValidationContext,
) -> Result<BlockImportResult, BlockImportError> {
    // Check block number and block hash against db
    let (block_number, parent_block_hash) =
        check_parent_hash_and_num(backend, block.header.parent_block_hash, block.unverified_block_number, &validation)?;

    // Update contract and its storage tries
    let global_state_root = update_tries(backend, &block, &validation, block_number)?;

    // Block hash
    let (block_hash, header) = block_hash(&block, &validation, block_number, parent_block_hash, global_state_root)?;

    tracing::debug!("verify_apply_inner store block {}", header.block_number);

    // store block, also uses rayon heavily internally
    backend
        .store_block(
            MadaraMaybePendingBlock {
                info: MadaraMaybePendingBlockInfo::NotPending(MadaraBlockInfo {
                    header: header.clone(),
                    block_hash,
                    // get tx hashes from receipts, they have been validated in pre_validate.
                    tx_hashes: block.receipts.iter().map(|tx| tx.transaction_hash()).collect(),
                }),
                inner: MadaraBlockInner { transactions: block.transactions, receipts: block.receipts },
            },
            block.state_diff,
            block.converted_classes,
            block.visited_segments,
            None,
        )
        .map_err(make_db_error("storing block in db"))?;

    Ok(BlockImportResult { header, block_hash })
}

/// See [`verify_apply_inner`].
pub fn verify_apply_pending_inner(
    backend: &MadaraBackend,
    block: PreValidatedPendingBlock,
    validation: BlockValidationContext,
) -> Result<PendingBlockImportResult, BlockImportError> {
    let (_block_number, parent_block_hash) =
        check_parent_hash_and_num(backend, block.header.parent_block_hash, None, &validation)?;

    let UnverifiedHeader {
        parent_block_hash: _,
        sequencer_address,
        block_timestamp,
        protocol_version,
        l1_gas_price,
        l1_da_mode,
    } = block.header;
    let header = PendingHeader {
        parent_block_hash,
        sequencer_address,
        block_timestamp,
        protocol_version,
        l1_gas_price,
        l1_da_mode,
    };

    backend
        .store_block(
            MadaraMaybePendingBlock {
                info: MadaraMaybePendingBlockInfo::Pending(MadaraPendingBlockInfo {
                    header: header.clone(),
                    tx_hashes: block.receipts.iter().map(|tx| tx.transaction_hash()).collect(),
                }),
                inner: MadaraBlockInner { transactions: block.transactions, receipts: block.receipts },
            },
            block.state_diff,
            block.converted_classes,
            block.visited_segments,
            None,
        )
        .map_err(make_db_error("storing block in db"))?;

    Ok(PendingBlockImportResult {})
}

fn make_db_error(context: impl Into<Cow<'static, str>>) -> impl FnOnce(MadaraStorageError) -> BlockImportError {
    move |error| BlockImportError::InternalDb { context: context.into(), error }
}

/// Returns the current block number and parent block hash.
fn check_parent_hash_and_num(
    backend: &MadaraBackend,
    parent_block_hash: Option<Felt>,
    unverified_block_number: Option<u64>,
    validation: &BlockValidationContext,
) -> Result<(u64, Felt), BlockImportError> {
    let latest_block_info =
        backend.get_block_info(&BlockId::Tag(BlockTag::Latest)).map_err(make_db_error("getting latest block info"))?;
    let (expected_block_number, expected_parent_block_hash) = if let Some(info) = latest_block_info {
        let info =
            info.as_nonpending().ok_or_else(|| BlockImportError::Internal("Latest block cannot be pending".into()))?;
        (info.header.block_number + 1, info.block_hash)
    } else {
        // importing genesis block
        (0, Felt::ZERO)
    };

    // TODO: this originally was checked after block number, but we need to check this first to detect a reorg.
    // TODO: this fn shouldn't be responsible for reorg detection anyway...
    if let Some(parent_block_hash) = parent_block_hash {
        if parent_block_hash != expected_parent_block_hash && !validation.ignore_block_order {
            return Err(BlockImportError::ParentHash { expected: expected_parent_block_hash, got: parent_block_hash });
        }
    }

    let block_number = if let Some(block_n) = unverified_block_number {
        if block_n != expected_block_number && !validation.ignore_block_order {
            return Err(BlockImportError::LatestBlockN { expected: expected_block_number, got: block_n });
        }
        block_n
    } else {
        expected_block_number
    };

    Ok((block_number, expected_parent_block_hash))
}

/// "STARKNET_STATE_V0"
const STARKNET_STATE_PREFIX: Felt = Felt::from_hex_unchecked("0x535441524b4e45545f53544154455f5630");

fn calculate_state_root(contracts_trie_root: Felt, classes_trie_root: Felt) -> Felt {
    if classes_trie_root == Felt::ZERO {
        contracts_trie_root
    } else {
        Poseidon::hash_array(&[STARKNET_STATE_PREFIX, contracts_trie_root, classes_trie_root])
    }
}

/// Returns the new global state root.
fn update_tries(
    backend: &MadaraBackend,
    block: &PreValidatedBlock,
    validation: &BlockValidationContext,
    block_number: u64,
) -> Result<Felt, BlockImportError> {
    if validation.trust_global_tries {
        let Some(global_state_root) = block.unverified_global_state_root else {
            return Err(BlockImportError::Internal(
                "Trying to import a block without a global state root when using trust_global_tries".into(),
            ));
        };
        return Ok(global_state_root);
    }

    tracing::debug!(
        "Deployed contracts: [{:?}]",
        block.state_diff.deployed_contracts.iter().map(|c| c.address.hex_display()).format(", ")
    );
    tracing::debug!(
        "Declared classes: [{:?}]",
        block.state_diff.declared_classes.iter().map(|c| c.class_hash.hex_display()).format(", ")
    );
    tracing::debug!(
        "Deprecated declared classes: [{:?}]",
        block.state_diff.deprecated_declared_classes.iter().map(|c| c.hex_display()).format(", ")
    );

    let (contract_trie_root, class_trie_root) = rayon::join(
        || {
            contracts::contract_trie_root(
                backend,
                &block.state_diff.deployed_contracts,
                &block.state_diff.replaced_classes,
                &block.state_diff.nonces,
                &block.state_diff.storage_diffs,
                block_number,
            )
        },
        || classes::class_trie_root(backend, &block.state_diff.declared_classes, block_number),
    );

    let state_root = calculate_state_root(
        contract_trie_root.map_err(make_db_error("updating contract trie root"))?,
        class_trie_root.map_err(make_db_error("updating class trie root"))?,
    );

    if let Some(expected) = block.unverified_global_state_root {
        if expected != state_root {
            return Err(BlockImportError::GlobalStateRoot { got: state_root, expected });
        }
    }

    Ok(state_root)
}

/// Returns the block hash and header.
fn block_hash(
    block: &PreValidatedBlock,
    validation: &BlockValidationContext,
    block_number: u64,
    parent_block_hash: Felt,
    global_state_root: Felt,
) -> Result<(Felt, Header), BlockImportError> {
    // Now, block hash

    let UnverifiedHeader {
        parent_block_hash: _,
        sequencer_address,
        block_timestamp,
        protocol_version,
        l1_gas_price,
        l1_da_mode,
    } = block.header.clone();

    let ValidatedCommitments {
        transaction_count,
        transaction_commitment,
        event_count,
        event_commitment,
        state_diff_length,
        state_diff_commitment,
        receipt_commitment,
    } = block.commitments;

    let header = Header {
        parent_block_hash,
        block_number,
        global_state_root,
        sequencer_address,
        block_timestamp,
        transaction_count,
        transaction_commitment,
        event_count,
        event_commitment,
        state_diff_length: Some(state_diff_length),
        state_diff_commitment: Some(state_diff_commitment),
        receipt_commitment: Some(receipt_commitment),
        protocol_version,
        l1_gas_price,
        l1_da_mode,
    };
    let block_hash = header.compute_hash(validation.chain_id.to_felt());

    if let Some(expected) = block.unverified_block_hash {
        // mismatched block hash is allowed for blocks 1466..=2242 on mainnet
        let is_special_trusted_case = validation.chain_id == ChainId::Mainnet && (1466..=2242).contains(&block_number);
        if is_special_trusted_case {
            return Ok((expected, header));
        }

        if expected != block_hash && !validation.ignore_block_order {
            return Err(BlockImportError::BlockHash { got: block_hash, expected });
        }
    }

    Ok((block_hash, header))
}

/// Reorgs the blockchain from its current tip back to `new_tip` from which a new fork can then be
/// played on top.
/// 
/// Returns the result of the reorg, which describes the part of the chain that was orphaned.
/// 
/// TODO: consider renaming this in order to distinguish between "reorg" and "reverting".
fn reorg(
    backend: &MadaraBackend,
    new_tip: &PreValidatedBlock, // TODO: we don't need a PreValidatedBlock, do we?
) -> Result<ReorgResult, BlockImportError> {

	let block_number = new_tip.unverified_block_number.expect("Can't reorg without block number");
    // TODO: we should ensure this exact block exists in our db
    let block_hash = new_tip.unverified_block_hash.expect("Can't reorg without a block hash");
	let block_id = BasicId::new(block_number);

    backend
		.contract_trie()
		.revert_to(block_id)
        .map_err(|error| BlockImportError::Internal(Cow::Owned(format!("error reverting contract trie: {}", error))))?;

    backend
		.contract_storage_trie()
		.revert_to(block_id)
        .map_err(|error| BlockImportError::Internal(Cow::Owned(format!("error reverting contract storage trie: {}", error))))?;

    backend
		.class_trie()
		.revert_to(block_id)
        .map_err(|error| BlockImportError::Internal(Cow::Owned(format!("error reverting class trie: {}", error))))?;

    backend.revert_to(block_number)
        .map_err(|error| BlockImportError::Internal(Cow::Owned(format!("error reverting block db: {}", error))))?;

    // TODO: ensure there is no race condition here...? (e.g. we reverted and someone else appends a block before this next call)
    let latest_block_info = backend
        .get_block_info(&BlockId::Tag(BlockTag::Latest))
        .map_err(make_db_error("getting latest block info"))?
        .ok_or(BlockImportError::Internal(Cow::Owned("no latest block after reorg".to_string())))?;

	Ok(ReorgResult {
        starting_block_hash: block_hash,
        starting_block_number: block_number,
        ending_block_hash: latest_block_info.block_hash().expect("how would a block not have a hash?"), // TODO: better error message, but srsly, how?
        ending_block_number: latest_block_info.block_n().expect("how would a block not have a block number?"), // TODO
	})

}

#[cfg(test)]
mod verify_apply_tests {
    use super::*;
    use crate::tests::block_import_utils::*;
    use mc_db::tests::common::{finalized_block_zero, finalized_state_diff_zero};

    use mp_chain_config::ChainConfig;

    use mp_state_update::{ContractStorageDiffItem, DeployedContractItem, StateDiff, StorageEntry};

    use rstest::*;
    use starknet_api::{core::ChainId, felt};
    use std::sync::Arc;

    /// Sets up a test backend.
    ///
    /// This function creates a new MadaraBackend instance with a test configuration, useful for isolated test environments.
    #[fixture]
    pub fn setup_test_backend() -> Arc<MadaraBackend> {
        let chain_config = Arc::new(ChainConfig::madara_test());
        MadaraBackend::open_for_testing(chain_config.clone())
    }

    /// Test various scenarios for the `check_parent_hash_and_num` function.
    ///
    /// This test covers different cases of block import, including:
    /// - Successful import of a regular block
    /// - Import of the genesis block
    /// - Handling mismatched block numbers
    /// - Handling mismatched parent hashes
    /// - Behavior when ignoring block order
    ///
    /// Each case tests the function's ability to correctly validate or reject
    /// block imports based on the given parameters and database state.
    #[rstest]
    #[case::success(
        Some(felt!("0x12345")),  // Parent block hash of the new block
        Some(2),                 // Block number of the new block
        false,                   // Not ignoring block order
        Ok((2, felt!("0x12345"))), // Expected result: success with correct block number and parent hash
        true                     // Populate DB with a previous block
    )]
    #[case::genesis_block(
        None,                    // No parent hash for genesis block
        None,                    // No specific block number (should default to 0)
        false,                   // Not ignoring block order
        Ok((0, felt!("0x0"))),   // Expected result: success with block 0 and zero parent hash
        false                    // Don't populate DB (simulating empty chain)
    )]
    #[case::mismatch_block_number(
        Some(felt!("0x12345")),  // Correct parent hash
        Some(3),                 // Incorrect block number (should be 2)
        false,                   // Not ignoring block order
        Err(BlockImportError::LatestBlockN { expected: 2, got: 3 }), // Expected error
        true                     // Populate DB with a previous block
    )]
    #[case::mismatch_parent_hash(
        Some(felt!("0x1")),      // Incorrect parent hash
        Some(2),                 // Correct block number
        false,                   // Not ignoring block order
        Err(BlockImportError::ParentHash { expected: felt!("0x12345"), got: felt!("0x1") }), // Expected error
        true                     // Populate DB with a previous block
    )]
    #[case::ignore_block_order(
        Some(felt!("0x1")),      // Incorrect parent hash (but will be ignored)
        Some(3),                 // Incorrect block number (but will be ignored)
        true,                    // Ignoring block order
        Ok((3, felt!("0x12345"))), // Expected result: success despite mismatches
        true                     // Populate DB with a previous block
    )]
    #[tokio::test]
    async fn test_check_parent_hash_and_num(
        #[case] parent_block_hash: Option<Felt>,
        #[case] unverified_block_number: Option<u64>,
        #[case] ignore_block_order: bool,
        #[case] expected_result: Result<(u64, Felt), BlockImportError>,
        #[case] populate_db: bool,
        setup_test_backend: Arc<MadaraBackend>,
    ) {
        // Set up a test backend (database)
        let backend = setup_test_backend;

        // Populate the database with a block in case it's not a genesis block
        if populate_db {
            let header = create_dummy_header();
            let pending_block = finalized_block_zero(header);
            backend.store_block(pending_block.clone(), finalized_state_diff_zero(), vec![], None, None).unwrap();
        }

        // Create a validation context with the specified ignore_block_order flag
        let validation = create_validation_context(ignore_block_order);

        // Call the function under test
        let result = check_parent_hash_and_num(&backend, parent_block_hash, unverified_block_number, &validation);

        // Assert that the result matches the expected outcome
        match (result, expected_result) {
            (Ok(actual), Ok(expected)) => assert_eq!(actual, expected),
            (Err(actual), Err(expected)) => assert_eq!(format!("{:?}", actual), format!("{:?}", expected)),
            _ => panic!("Result types do not match"),
        }
    }

    /// Test cases for the `calculate_state_root` function.
    ///
    /// This test uses `rstest` to parameterize different scenarios for calculating
    /// the state root. It verifies that the function correctly handles various
    /// input combinations and produces the expected results.
    #[rstest]
    #[case::non_zero_inputs(
            felt!("0x123456"),  // Non-zero contracts trie root
            felt!("0x789abc"),  // Non-zero classes trie root
            // Expected result: Poseidon hash of STARKNET_STATE_PREFIX and both non-zero roots
            felt!("0x6beb971880d4b4996b10fe613b8d49fa3dda8f8b63156c919077e08c534d06e")
        )]
    #[case::zero_class_trie_root(
            felt!("0x123456"),  // Non-zero contracts trie root
            felt!("0x0"),       // Zero classes trie root
            felt!("0x123456")   // Expected result: same as contracts trie root
        )]
    fn test_calculate_state_root(
        #[case] contracts_trie_root: Felt,
        #[case] classes_trie_root: Felt,
        #[case] expected_result: Felt,
    ) {
        // GIVEN: We have a contracts trie root and a classes trie root

        // WHEN: We calculate the state root using these inputs
        let result = calculate_state_root(contracts_trie_root, classes_trie_root);

        // THEN: The calculated state root should match the expected result
        assert_eq!(result, expected_result, "State root should match the expected result");
    }

    /// Test cases for the `update_tries` function.
    ///
    /// This test uses `rstest` to parameterize different scenarios for updating the tries.
    /// It verifies that the function correctly handles various input combinations and
    /// produces the expected results or errors.
    #[rstest]
    #[case::success(
            // A non-zero global state root
            Some(felt!("0x738e796f750b21ddb3ce528ca88f7e35fad580768bd58571995b19a6809bb4a")),
            // A non-empty state diff with deployed contracts and storage changes
            StateDiff {
                deployed_contracts: vec![(DeployedContractItem { address: felt!("0x1"), class_hash: felt!("0x1") })],
                storage_diffs: vec![
                    (ContractStorageDiffItem {
                        address: felt!("0x1"),
                        storage_entries: vec![(StorageEntry { key: felt!("0x1"), value: felt!("0x1") })],
                    }),
                ],
                ..Default::default()
            },
            false, // Don't trust global tries
            // Expected result: the same as the input global state root
            Ok(felt!("0x738e796f750b21ddb3ce528ca88f7e35fad580768bd58571995b19a6809bb4a"))
        )]
    #[case::trust_global_tries(
            Some(felt!("0xa")), // A non-zero global state root
            StateDiff::default(), // Empty state diff
            true, // Trust global tries
            Ok(felt!("0xa")) // Expected result: the same as the input global state root
        )]
    #[case::missing_global_state_root(
            None, // Missing global state root
            StateDiff::default(), // Empty state diff
            true, // Trust global tries (irrelevant in this case)
            // Expected result: an Internal error
            Err(BlockImportError::Internal("Trying to import a block without a global state root when using trust_global_tries".into()))
        )]
    #[case::mismatch_global_state_root(
            Some(felt!("0xb")), // A non-zero global state root
            StateDiff::default(), // Empty state diff
            false, // Don't trust global tries
            // Expected result: a GlobalStateRoot error due to mismatch
            Err(BlockImportError::GlobalStateRoot { got: felt!("0x0"), expected: felt!("0xb") })
        )]
    #[case::empty_state_diff(
            Some(felt!("0x0")), // Zero global state root
            StateDiff::default(), // Empty state diff
            false, // Don't trust global tries
            Ok(felt!("0x0")) // Expected result: zero global state root
        )]
    #[tokio::test]
    async fn test_update_tries(
        #[case] unverified_global_state_root: Option<Felt>,
        #[case] state_diff: StateDiff,
        #[case] trust_global_tries: bool,
        #[case] expected_result: Result<Felt, BlockImportError>,
        setup_test_backend: Arc<MadaraBackend>,
    ) {
        // GIVEN: We have a test backend and a block with specified parameters
        let backend = setup_test_backend;
        let mut block = create_dummy_block();
        block.unverified_global_state_root = unverified_global_state_root;
        block.state_diff = state_diff;

        // AND: We have a validation context with specified trust_global_tries
        let validation = BlockValidationContext {
            chain_id: ChainId::Mainnet,
            ignore_block_order: false,
            trust_global_tries,
            trust_transaction_hashes: false,
            trust_class_hashes: false,
        };

        // WHEN: We call update_tries with these parameters
        let result = update_tries(&backend, &block, &validation, 1);

        // THEN: The result should match the expected outcome
        match (result, expected_result) {
            (Ok(got), Ok(expected)) => {
                // For successful cases, compare the returned Felt values
                assert_eq!(got, expected, "Returned global state root should match expected value");
            }
            (Err(got), Err(expected)) => {
                // For error cases, compare the string representations of the errors
                assert_eq!(
                    format!("{:?}", got),
                    format!("{:?}", expected),
                    "Returned error should match expected error"
                );
            }
            _ => panic!("Result types do not match"),
        }
    }

    #[rstest]
    // Case 1: Successful block hash calculation
    #[case::success(
            create_dummy_block(),
            create_validation_context(false),
            1,
            felt!("0x1"),
            felt!("0xa"),
            Ok((felt!("0x271814f105da644661d0ef938cfccfd66d3e3585683fbcbee339db3d29c4574"), 1))
        )]
    // Case 2: Block hash mismatch
    #[case::mismatch(
            {
                let mut block = create_dummy_block();
                block.unverified_block_hash = Some(felt!("0xdeadbeef"));
                block
            },
            create_validation_context(false),
            1,
            felt!("0x1"),
            felt!("0xa"),
            Err(BlockImportError::BlockHash {
                got: felt!("0x271814f105da644661d0ef938cfccfd66d3e3585683fbcbee339db3d29c4574"),
                expected: felt!("0xdeadbeef")
            })
        )]
    // Case 3: Special trusted case for Mainnet blocks 1466-2242
    #[case::special_trusted_case(
            {
                let mut block = create_dummy_block();
                block.unverified_block_hash = Some(felt!("0xdeadbeef"));
                block.unverified_block_number = Some(1466);
                block
            },
            BlockValidationContext {
                chain_id: ChainId::Mainnet,
                ignore_block_order: false,
                trust_global_tries: false,
                trust_transaction_hashes: false,
                trust_class_hashes: false,
            },
            1466,
            felt!("0x1"),
            felt!("0xa"),
            Ok((felt!("0xdeadbeef"), 1466))
        )]
    fn test_block_hash(
        #[case] block: PreValidatedBlock,
        #[case] validation: BlockValidationContext,
        #[case] block_number: u64,
        #[case] parent_block_hash: Felt,
        #[case] global_state_root: Felt,
        #[case] expected_result: Result<(Felt, u64), BlockImportError>,
    ) {
        // This test function verifies the behavior of the block_hash function
        // under various scenarios. It uses parameterized testing to cover
        // different cases:
        //
        // 1. Success case:
        //    - Input: A default dummy block with standard validation context
        //    - Expected: Successful calculation of block hash and number
        //    - Purpose: Verifies correct hash calculation for a valid block
        //
        // 2. Mismatch case:
        //    - Input: A block with a predefined unverified hash that doesn't match the calculated hash
        //    - Expected: BlockImportError indicating hash mismatch
        //    - Purpose: Ensures the function detects and reports hash mismatches correctly
        //
        // 3. Special trusted case: (ideally this should be removed)
        //    - Input: A block simulating Mainnet blocks 1466-2242 with a mismatched hash
        //    - Expected: Successful result, accepting the provided unverified hash
        //    - Purpose: Verifies the special handling for a specific range of Mainnet blocks

        // GIVEN: We have a block, validation context, block number, parent block hash, and global state root

        // WHEN: We call the block_hash function with these parameters
        let result = block_hash(&block, &validation, block_number, parent_block_hash, global_state_root);

        // THEN: We compare the result with the expected outcome
        match (result, expected_result) {
            (Ok((hash, header)), Ok((expected_hash, expected_block_number))) => {
                // For successful cases:
                // - Verify that the calculated hash matches the expected hash
                // - Check if the block number in the header matches the expected block number
                assert_eq!(hash, expected_hash, "Block hash should match");
                assert_eq!(header.block_number, expected_block_number, "Block number should match");
            }
            (Err(got), Err(expected)) => {
                // For error cases:
                // - Compare the string representations of the actual and expected errors
                // This ensures that not only the error type matches, but also the error details
                assert_eq!(format!("{:?}", got), format!("{:?}", expected), "Errors should match");
            }
            _ => panic!("Result types do not match."),
        }
    }

    mod verify_apply_inner_tests {
        use super::*;

        /// Test successful block verification and storage.
        ///
        /// Verifies that:
        /// 1. The function correctly verifies and stores a new pending block.
        /// 2. The latest block number is updated after successful storage.
        #[rstest]
        #[tokio::test]
        async fn test_verify_apply_inner_success_stores_block(setup_test_backend: Arc<MadaraBackend>) {
            let backend = setup_test_backend;
            let mut header = create_dummy_header();
            header.block_number = 0;
            let pending_block = finalized_block_zero(header);
            backend.store_block(pending_block.clone(), finalized_state_diff_zero(), vec![], None, None).unwrap();

            assert_eq!(backend.get_latest_block_n().unwrap(), Some(0));

            let mut block = create_dummy_block();
            block.header.parent_block_hash = Some(felt!("0x12345"));
            block.unverified_global_state_root = Some(felt!("0x0"));
            let validation = create_validation_context(false);

            let _result = verify_apply_inner(&backend, block, validation.clone());

            assert_eq!(backend.get_latest_block_n().unwrap(), Some(1));
        }

        /// Test error handling during block verification.
        ///
        /// Verifies that:
        /// 1. The function returns an error for invalid block data (e.g., mismatched block number).
        /// 2. The latest block number remains unchanged when an error occurs.
        #[rstest]
        #[tokio::test]
        async fn test_verify_apply_inner_error_does_not_store_block(setup_test_backend: Arc<MadaraBackend>) {
            let backend = setup_test_backend;
            let mut header = create_dummy_header();
            header.block_number = 0;
            let pending_block = finalized_block_zero(header);
            backend.store_block(pending_block.clone(), finalized_state_diff_zero(), vec![], None, None).unwrap();

            assert_eq!(backend.get_latest_block_n().unwrap(), Some(0));

            let mut block = create_dummy_block();
            block.header.parent_block_hash = Some(felt!("0x12345"));
            block.unverified_block_number = Some(2); // Mismatch to trigger an error

            let validation = create_validation_context(false);

            let result = verify_apply_inner(&backend, block, validation);

            assert!(matches!(result.unwrap_err(), BlockImportError::LatestBlockN { .. }));
            assert_eq!(backend.get_latest_block_n().unwrap(), Some(0));
        }
    }

    mod verify_apply_pending_tests {
        use mc_db::db_block_id::DbBlockId;

        use super::*;
        const BLOCK_ID_PENDING: DbBlockId = DbBlockId::Pending;

        /// Test successful pending block verification and storage.
        ///
        /// Verifies that:
        /// 1. The function correctly verifies and stores a new block.
        /// 2. The latest block number is updated after successful storage.
        #[rstest]
        #[tokio::test]
        async fn test_verify_apply_pending_success_stores_block(setup_test_backend: Arc<MadaraBackend>) {
            // Setup
            let backend = setup_test_backend;
            let mut genesis_header = create_dummy_header();
            genesis_header.block_number = 0;
            let genesis_block = finalized_block_zero(genesis_header.clone());
            backend.store_block(genesis_block, finalized_state_diff_zero(), vec![], None, None).unwrap();

            assert_eq!(backend.get_latest_block_n().unwrap(), Some(0));

            // Create pending block
            let mut pending_block = create_dummy_pending_block();
            pending_block.header.parent_block_hash = Some(felt!("0x12345"));
            let validation_context = create_validation_context(false);

            // Expected pending header
            let expected_pending_header = PendingHeader {
                parent_block_hash: felt!("0x12345"),
                sequencer_address: genesis_header.sequencer_address,
                block_timestamp: genesis_header.block_timestamp,
                protocol_version: genesis_header.protocol_version,
                l1_gas_price: genesis_header.l1_gas_price,
                l1_da_mode: genesis_header.l1_da_mode,
            };

            // Expected pending block info
            let expected_pending_info = MadaraMaybePendingBlockInfo::Pending(MadaraPendingBlockInfo {
                header: expected_pending_header,
                tx_hashes: pending_block.receipts.iter().map(|tx| tx.transaction_hash()).collect(),
            });

            // Verify and apply pending block
            verify_apply_pending_inner(&backend, pending_block, validation_context).unwrap();

            // Assert
            let stored_pending_info = backend.get_block_info(&BLOCK_ID_PENDING).unwrap().unwrap();
            assert_eq!(stored_pending_info, expected_pending_info);
        }

        /// Test error handling during pending block verification.
        ///
        /// Verifies that:
        /// 1. The function returns an error for invalid block data (e.g., mismatched block number).
        /// 2. The latest block number remains unchanged when an error occurs.
        #[rstest]
        #[tokio::test]
        async fn test_verify_apply_pending_error_does_not_store_block(setup_test_backend: Arc<MadaraBackend>) {
            // Setup
            let backend = setup_test_backend;
            let mut genesis_header = create_dummy_header();
            genesis_header.block_number = 0;
            let genesis_block = finalized_block_zero(genesis_header.clone());
            backend.store_block(genesis_block, finalized_state_diff_zero(), vec![], None, None).unwrap();

            assert_eq!(backend.get_latest_block_n().unwrap(), Some(0));

            // Create pending block with mismatched parent hash
            let mut pending_block = create_dummy_pending_block();
            pending_block.header.parent_block_hash = Some(felt!("0x1234")); // Mismatched parent hash
            let validation_context = create_validation_context(false);

            // Expected pending header (should not be stored)
            let unexpected_pending_header = PendingHeader {
                parent_block_hash: felt!("0x1234"),
                sequencer_address: genesis_header.sequencer_address,
                block_timestamp: genesis_header.block_timestamp,
                protocol_version: genesis_header.protocol_version,
                l1_gas_price: genesis_header.l1_gas_price,
                l1_da_mode: genesis_header.l1_da_mode,
            };

            // Expected pending block info (should not be stored)
            let unexpected_pending_info = MadaraMaybePendingBlockInfo::Pending(MadaraPendingBlockInfo {
                header: unexpected_pending_header,
                tx_hashes: pending_block.receipts.iter().map(|tx| tx.transaction_hash()).collect(),
            });

            // Verify and apply pending block (should fail)
            let result = verify_apply_pending_inner(&backend, pending_block, validation_context);

            // Assert
            assert!(result.is_err(), "Expected an error due to mismatched parent hash");

            let stored_pending_info = backend.get_block_info(&BLOCK_ID_PENDING).unwrap();
            assert_ne!(
                stored_pending_info,
                Some(unexpected_pending_info),
                "Unexpected pending block should not be stored"
            );
        }
    }

    mod reorg_tests {
        use super::*;

        /// This struct provides the inputs to the reorg tests. It does so based on integers
        /// representing a few different lengths of different parts of a forked chain, where one
        /// side of the fork is first created, a reorg occurs, and then the other side of the fork
        /// is created.
        /// 
        /// Consider this diagram:
        /// 
        ///         O        ---
        ///         |          | "original_chain_length"
        ///         O          | This part of the chain is not reorged, the last block is the parent
        ///         |          | of the fork that we reorg across
        ///         O        ---
        ///        / \
        ///       /   \
        ///      N     X
        ///      |     |
        ///      N     X     --- the "X" chain is added and later removed when we reorg, its length
        ///      |               is "orphaned_chain_length"
        ///      N
        ///      |
        ///      N           --- the "N" chain is the new chain which is created after we reorg back
        ///                      to the end of the "O" part of the chain, its length is
        ///                      "new_chain_length"
        struct ReorgTestArgs {
            /// original chain length, including block 0 (e.g. "1" means only genesis block).
            /// must be > 0.
            original_chain_length: u64,
            /// the length of the orphaned chain
            orphaned_chain_length: u64,
            /// the length of the new chain
            new_chain_length: u64,
            /// how many passes of reorgs to make. each pass does:
            /// * append `orphaned_chain_length` blocks to the tip
            /// * revert back to tip
            /// * append `new_chain_length` blocks
            passes: u64,
        }

        #[rstest]
        #[case::reorg_from_genesis(
            ReorgTestArgs{
                original_chain_length: 1,
                orphaned_chain_length: 2,
                new_chain_length: 1,
                passes: 1,
            },
        )]
        #[case::empty_orphan_reorg(
            ReorgTestArgs{
                original_chain_length: 1,
                orphaned_chain_length: 0,
                new_chain_length: 1,
                passes: 1,
            },
        )]
        #[case::success(
            ReorgTestArgs{
                original_chain_length: 6,
                orphaned_chain_length: 3,
                new_chain_length: 2,
                passes: 1,
            },
        )]
        #[case::multiple_passes(
            ReorgTestArgs{
                original_chain_length: 6,
                orphaned_chain_length: 3,
                new_chain_length: 2,
                passes: 5,
            },
        )]
        #[case::multiple_passes_no_new_chain(
            ReorgTestArgs{
                original_chain_length: 6,
                orphaned_chain_length: 3,
                new_chain_length: 0,
                passes: 5,
            },
        )]
        #[case::multiple_passes_repetitive_fork(
            ReorgTestArgs{
                original_chain_length: 2,
                orphaned_chain_length: 2,
                new_chain_length: 2,
                passes: 5,
            },
        )]
        #[case::deep_reorg(
            ReorgTestArgs{
                original_chain_length: 32,
                orphaned_chain_length: 32,
                new_chain_length: 32,
                passes: 1,
            },
        )]
        #[tokio::test]
        async fn test_reorg(
            setup_test_backend: Arc<MadaraBackend>,
            #[case] args: ReorgTestArgs,
        ) {
            let backend = setup_test_backend;
            let validation = create_validation_context(false);

            // utility fn to append an empty block to the given parent block, returning the new block's hash
            let append_empty_block = |new_block_height: u64, parent_hash: Option<Felt>| -> (Felt, PreValidatedBlock) {
                println!("attempting to add block {}", new_block_height);
                let mut block = create_dummy_block();
                block.unverified_block_number = Some(new_block_height);
                block.unverified_global_state_root = Some(felt!("0x0"));
                block.header.parent_block_hash = parent_hash;
                let block_import = verify_apply_inner(&backend, block.clone(), validation.clone()).expect("verify_apply_inner failed");
                println!("added block {} (0x{:x})", new_block_height, block_import.block_hash);

                block.unverified_block_hash = Some(block_import.block_hash);

                (block_import.block_hash, block)
            };

            // create the original chain
            println!("-----------------------");
            println!("creating original chain (length: {})", args.original_chain_length);
            let mut parent_hash = None;
            let mut reorg_parent_block = None;
            assert!(
                args.original_chain_length > 0,
                "Cannot create an empty chain, we always need at least genesis"
            );
            for i in 0..args.original_chain_length {
                let (new_block_hash, new_parent_block) = append_empty_block(i, parent_hash);
                parent_hash = Some(new_block_hash);
                reorg_parent_block = Some(new_parent_block);
            }
            let mut reorg_parent_hash = parent_hash.clone().expect(
                "logic error: we should have created at least one block which is our parent");
            
            let mut parent_height = args.original_chain_length;
            assert!(args.passes > 0);
            for _ in 0..args.passes {

                // build a soon-to-be-orphaned chain on top of the original
                println!("-----------------------");
                println!("creating orphaned chain (length: {})", args.orphaned_chain_length);
                for _ in 0..args.orphaned_chain_length {
                    let (new_block_hash, _) = append_empty_block(parent_height, parent_hash);
                    parent_hash = Some(new_block_hash);
                    parent_height += 1;
                }

                println!("-----------------------");
                println!("Reorging back to parent {} ({:?})", parent_height, reorg_parent_hash);

                let _ = reorg(&backend, &reorg_parent_block.clone().expect("Should have a parent by now")).expect("reorg failed");
                parent_height -= args.orphaned_chain_length;
                // TODO: need to reset parent hash here, we can get this by having reorg() properly return to/from values

                println!("-----------------------");
                println!("Extending reorg'ed chain on top of {} ({:?})", args.original_chain_length, reorg_parent_hash);

                // reorg after given parent (start with 1 since we already added our reorg block)
                parent_hash = Some(reorg_parent_hash);
                for _ in 0..args.new_chain_length {
                    let (new_block_hash, new_block) = append_empty_block(parent_height, parent_hash);
                    parent_hash = Some(new_block_hash);
                    parent_height += 1;

                    // next iteration we will want to reorg back to this point
                    reorg_parent_block = Some(new_block);
                }
                reorg_parent_hash = parent_hash.expect("parent_hash should be set by now");

                let latest_block_n = backend.get_latest_block_n().expect("get_latest_block_n() failed").expect("latest_block_n is None");
                // assert_eq!(args.original_chain_length + args.new_chain_length - 1, latest_block_n); // doesn't make sense with multiple passes
                let latest_block_hash = backend.get_block_hash(&BlockId::Number(latest_block_n)).expect("get_block_hash failed after reorg");
                assert_eq!(latest_block_hash, parent_hash);
            }
        }
    }
}
