use std::{borrow::Cow, sync::Arc};

use mc_db::{MadaraBackend, MadaraStorageError};
use mp_block::{
    header::PendingHeader, BlockId, BlockTag, Header, MadaraBlockInfo, MadaraBlockInner, MadaraMaybePendingBlock,
    MadaraMaybePendingBlockInfo, MadaraPendingBlockInfo,
};
use mp_convert::ToFelt;
use starknet_api::core::ChainId;
use starknet_core::types::Felt;
use starknet_types_core::hash::{Poseidon, StarkHash};

use crate::{
    BlockImportError, BlockImportResult, BlockValidationContext, PendingBlockImportResult, PreValidatedBlock,
    PreValidatedPendingBlock, RayonPool, UnverifiedHeader, ValidatedCommitments,
};

mod classes;
mod contracts;

pub struct VerifyApply {
    pool: Arc<RayonPool>,
    backend: Arc<MadaraBackend>,
    // Only one thread at once can verify_apply. This is the update trie step cannot be parallelized over blocks, and in addition
    // our database does not support concurrent write access.
    mutex: tokio::sync::Mutex<()>,
}

impl VerifyApply {
    pub fn new(backend: Arc<MadaraBackend>, pool: Arc<RayonPool>) -> Self {
        Self { pool, backend, mutex: Default::default() }
    }

    /// This function wraps the [`verify_apply_inner`] step, which runs on the rayon pool, in a tokio-friendly future.
    pub async fn verify_apply(
        &self,
        block: PreValidatedBlock,
        validation: BlockValidationContext,
    ) -> Result<BlockImportResult, BlockImportError> {
        let _exclusive = self.mutex.lock().await;

        let backend = Arc::clone(&self.backend);
        self.pool.spawn_rayon_task(move || verify_apply_inner(&backend, block, validation)).await
    }

    /// See [`Self::verify_apply`].
    pub async fn verify_apply_pending(
        &self,
        block: PreValidatedPendingBlock,
        validation: BlockValidationContext,
    ) -> Result<PendingBlockImportResult, BlockImportError> {
        let _exclusive = self.mutex.lock().await;

        let backend = Arc::clone(&self.backend);
        self.pool.spawn_rayon_task(move || verify_apply_pending_inner(&backend, block, validation)).await
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

    log::debug!("verify_apply_inner store block {}", header.block_number);

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

    let block_number = if let Some(block_n) = unverified_block_number {
        if block_n != expected_block_number && !validation.ignore_block_order {
            return Err(BlockImportError::LatestBlockN { expected: expected_block_number, got: block_n });
        }
        block_n
    } else {
        expected_block_number
    };

    if let Some(parent_block_hash) = parent_block_hash {
        if parent_block_hash != expected_parent_block_hash && !validation.ignore_block_order {
            return Err(BlockImportError::ParentHash { expected: expected_parent_block_hash, got: parent_block_hash });
        }
    }

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
    println!("Checkpoint 1: Checking if we trust global tries");
    if validation.trust_global_tries {
        let Some(global_state_root) = block.unverified_global_state_root else {
            return Err(BlockImportError::Internal("Trying to import a block without a global state root but ".into()));
        };
        println!("Checkpoint 2: Returning global state root");
        return Ok(global_state_root);
    }

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
        state_diff_length,
        state_diff_commitment,
        receipt_commitment,
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

#[cfg(test)]
mod verify_apply_tests {
    use super::*;
    use mc_db::DatabaseService;
    use mp_block::header::{GasPrices, L1DataAvailabilityMode};
    use mp_block::Header;
    use mp_chain_config::{ChainConfig, StarknetVersion};
    use mp_receipt::{
        DeclareTransactionReceipt, DeployAccountTransactionReceipt, DeployTransactionReceipt, InvokeTransactionReceipt,
        L1HandlerTransactionReceipt,
    };
    use mp_state_update::StateDiff;
    use mp_transactions::{
        DeclareTransactionV1, DeployAccountTransactionV3, DeployTransaction, InvokeTransactionV1, L1HandlerTransaction,
    };
    use starknet_api::{core::ChainId, felt};
    use std::sync::Arc;
    use tempfile::TempDir;

    /// Creates a dummy UnverifiedHeader for testing purposes.
    ///
    /// This function generates an UnverifiedHeader with predefined values,
    /// useful for creating consistent test scenarios.
    fn create_dummy_unverified_header() -> UnverifiedHeader {
        UnverifiedHeader {
            parent_block_hash: Some(felt!("0x1")),
            sequencer_address: felt!("0x2"),
            block_timestamp: 12345,
            protocol_version: StarknetVersion::new(0, 13, 2, 0),
            l1_gas_price: GasPrices {
                eth_l1_gas_price: 14,
                strk_l1_gas_price: 15,
                eth_l1_data_gas_price: 16,
                strk_l1_data_gas_price: 17,
            },
            l1_da_mode: L1DataAvailabilityMode::Blob,
        }
    }

    /// Creates dummy ValidatedCommitments for testing purposes.
    ///
    /// This function generates ValidatedCommitments with predefined values,
    /// useful for creating consistent test scenarios.
    fn create_dummy_commitments() -> ValidatedCommitments {
        ValidatedCommitments {
            transaction_count: 1,
            transaction_commitment: felt!("0x6"),
            event_count: 2,
            event_commitment: felt!("0x7"),
            state_diff_length: 3,
            state_diff_commitment: felt!("0x8"),
            receipt_commitment: felt!("0x9"),
        }
    }

    /// Sets up a test backend with a temporary directory.
    ///
    /// This function creates a new MadaraBackend instance with a temporary
    /// directory and test configuration, useful for isolated test environments.
    async fn setup_test_backend() -> Arc<MadaraBackend> {
        let temp_dir = TempDir::new().unwrap();
        let chain_config = Arc::new(ChainConfig::test_config());
        let db_service = DatabaseService::new(temp_dir.path(), None, false, chain_config).await.unwrap();
        db_service.backend().clone()
    }

    /// Creates a BlockValidationContext for testing purposes.
    ///
    /// This function generates a BlockValidationContext with customizable
    /// ignore_block_order flag, useful for testing different validation scenarios.
    fn create_validation_context(ignore_block_order: bool) -> BlockValidationContext {
        BlockValidationContext {
            chain_id: ChainId::Other("something".to_string()),
            ignore_block_order,
            trust_global_tries: false,
            trust_transaction_hashes: false,
            trust_class_hashes: false,
        }
    }

    /// Creates a dummy Header for testing purposes.
    ///
    /// This function generates a Header with predefined values,
    /// useful for creating consistent test scenarios.
    fn create_dummy_header() -> Header {
        Header {
            parent_block_hash: felt!("0x1"),
            sequencer_address: felt!("0x2"),
            block_number: 1,
            global_state_root: felt!("0xa"),
            transaction_count: 0,
            transaction_commitment: Felt::ZERO,
            event_count: 0,
            event_commitment: Felt::ZERO,
            state_diff_length: 0,
            state_diff_commitment: Felt::ZERO,
            receipt_commitment: Felt::ZERO,
            block_timestamp: 12345,
            protocol_version: StarknetVersion::new(0, 13, 2, 0),
            l1_gas_price: GasPrices {
                eth_l1_gas_price: 14,
                strk_l1_gas_price: 15,
                eth_l1_data_gas_price: 16,
                strk_l1_data_gas_price: 17,
            },
            l1_da_mode: L1DataAvailabilityMode::Blob,
        }
    }

    /// Creates a finalized block zero (genesis block) for testing purposes.
    ///
    /// This function generates a MadaraMaybePendingBlock representing the genesis block,
    /// useful for testing scenarios involving the first block in the chain.
    fn finalized_block_zero(header: Header) -> MadaraMaybePendingBlock {
        let transactions = vec![
            InvokeTransactionV1::default().into(),
            L1HandlerTransaction::default().into(),
            DeclareTransactionV1::default().into(),
            DeployTransaction::default().into(),
            DeployAccountTransactionV3::default().into(),
        ];

        let transaction_receipts = vec![
            InvokeTransactionReceipt::default().into(),
            L1HandlerTransactionReceipt::default().into(),
            DeclareTransactionReceipt::default().into(),
            DeployTransactionReceipt::default().into(),
            DeployAccountTransactionReceipt::default().into(),
        ];

        let block_inner = MadaraBlockInner::new(transactions, transaction_receipts);

        let tx_hashes = vec![Felt::from(0), Felt::from(1), Felt::from(2), Felt::from(3), Felt::from(4)];
        let block_info = MadaraBlockInfo::new(header, tx_hashes, felt!("0x12345"));

        MadaraMaybePendingBlock { info: block_info.into(), inner: block_inner }
    }

    /// Creates an empty StateDiff for testing purposes.
    ///
    /// This function returns a default StateDiff, useful for testing scenarios
    /// where an empty state difference is needed.
    fn finalized_state_diff_zero() -> StateDiff {
        StateDiff::default()
    }

    pub mod check_parent_hash_and_num_tests {
        use super::*;

        /// Test successful parent hash and number check.
        ///
        /// Verifies that:
        /// 1. The function correctly validates the parent hash and block number.
        /// 2. It returns the expected block number and parent hash for a valid input.
        #[tokio::test]
        async fn test_check_parent_hash_and_num_success() {
            let backend = setup_test_backend().await;
            let mut header = create_dummy_header();

            let new_block_number = 2;

            header.block_number = new_block_number;

            let pending_block = finalized_block_zero(header);
            let block_hash = pending_block.info.block_hash();
            backend.store_block(pending_block.clone(), finalized_state_diff_zero(), vec![]).unwrap();

            let validation = create_validation_context(false);

            let result = check_parent_hash_and_num(&backend, block_hash, Some(new_block_number + 1), &validation)
                .expect("Failed to check parent hash and number");
            assert_eq!(result, (new_block_number + 1, block_hash.unwrap()));
        }

        /// Test parent hash and number check for genesis block.
        ///
        /// Verifies that:
        /// 1. The function correctly handles the case of importing a genesis block.
        /// 2. It returns the expected values (0, Felt::ZERO) for a genesis block.
        #[tokio::test]
        async fn test_check_parent_hash_and_num_genesis_block() {
            let backend = setup_test_backend().await;
            let parent_block_hash = None;
            let unverified_block_number = None;
            let validation = create_validation_context(false);

            let result = check_parent_hash_and_num(&backend, parent_block_hash, unverified_block_number, &validation)
                .expect("Failed to check parent hash and number");
            assert_eq!(result, (0, felt!("0x0")));
        }

        /// Test error handling for mismatched block number.
        ///
        /// Verifies that:
        /// 1. The function detects a mismatch between the expected and provided block number.
        /// 2. It returns a LatestBlockN error when there's a mismatch.
        #[tokio::test]
        async fn test_check_parent_hash_and_num_mismatch_block_number() {
            let backend = setup_test_backend().await;
            let header = create_dummy_header();

            let pending_block = finalized_block_zero(header);
            backend.store_block(pending_block.clone(), finalized_state_diff_zero(), vec![]).unwrap();

            let parent_block_hash = pending_block.info.block_hash();
            let unverified_block_number = pending_block.info.block_n().map(|n| n + 2); // Mismatch
            let validation = create_validation_context(false);

            let result = check_parent_hash_and_num(&backend, parent_block_hash, unverified_block_number, &validation);
            assert!(matches!(result, Err(BlockImportError::LatestBlockN { .. })));
        }

        /// Test error handling for mismatched parent hash.
        ///
        /// Verifies that:
        /// 1. The function detects a mismatch between the expected and provided parent hash.
        /// 2. It returns a ParentHash error when there's a mismatch.
        #[tokio::test]
        async fn test_check_parent_hash_and_num_mismatch_parent_hash() {
            let backend = setup_test_backend().await;
            let header = create_dummy_header();
            let pending_block = finalized_block_zero(header);
            backend.store_block(pending_block.clone(), finalized_state_diff_zero(), vec![]).unwrap();

            let parent_block_hash = Some(felt!("0x1")); // Mismatch
            let unverified_block_number = pending_block.info.block_n().map(|n| n + 1);
            let validation = create_validation_context(false);

            let result = check_parent_hash_and_num(&backend, parent_block_hash, unverified_block_number, &validation);
            assert!(matches!(result, Err(BlockImportError::ParentHash { .. })));
        }

        /// Test ignore_block_order flag functionality.
        ///
        /// Verifies that:
        /// 1. When ignore_block_order is true, mismatches in block number and parent hash are allowed.
        /// 2. The function returns the provided values instead of throwing errors.
        #[tokio::test]
        async fn test_check_parent_hash_and_num_ignore_block_order() {
            let backend = setup_test_backend().await;
            let header = create_dummy_header();
            let pending_block = finalized_block_zero(header);
            backend.store_block(pending_block.clone(), finalized_state_diff_zero(), vec![]).unwrap();

            let parent_block_hash = Some(felt!("0x1")); // Mismatch
            let unverified_block_number = pending_block.info.block_n().map(|n| n + 2); // Mismatch
            let validation = create_validation_context(true);

            let result = check_parent_hash_and_num(&backend, parent_block_hash, unverified_block_number, &validation)
                .expect("Failed to check parent hash and number");
            assert_eq!(result, (pending_block.info.block_n().unwrap() + 2, pending_block.info.block_hash().unwrap()));
        }
    }

    mod calculate_state_root_tests {
        use starknet_api::felt;

        use super::*;

        /// Test successful state root calculation.
        ///
        /// Verifies that:
        /// 1. The function correctly calculates the state root from non-zero inputs.
        /// 2. The result is verified and differs from both input roots.
        #[test]
        fn test_calculate_state_root_success() {
            let contracts_trie_root = felt!("0x123456");
            let classes_trie_root = felt!("0x789abc");

            let result = calculate_state_root(contracts_trie_root, classes_trie_root);

            assert_eq!(
                result,
                felt!("0x6beb971880d4b4996b10fe613b8d49fa3dda8f8b63156c919077e08c534d06e"),
                "State root should match"
            );
            assert_ne!(result, contracts_trie_root, "State root should not be equal to contracts_trie_root");
            assert_ne!(result, classes_trie_root, "State root should not be equal to classes_trie_root");
        }

        /// Test state root calculation with zero class trie root.
        ///
        /// Verifies that:
        /// 1. When the class trie root is zero, the function returns the contracts trie root.
        #[test]
        fn test_calculate_state_root_zero_class_trie_root() {
            let contracts_trie_root = felt!("0x123456");
            let classes_trie_root = felt!("0x0");

            let result = calculate_state_root(contracts_trie_root, classes_trie_root);

            assert_eq!(
                result, contracts_trie_root,
                "State root should be equal to contracts_trie_root when classes_trie_root is zero"
            );
        }
    }

    mod update_tries_tests {
        use super::*;
        use mp_state_update::{ContractStorageDiffItem, DeployedContractItem, StateDiff, StorageEntry};

        /// Creates a dummy PreValidatedBlock for testing purposes.
        ///
        /// This function generates a PreValidatedBlock with predefined values,
        /// useful for testing update_tries scenarios.
        fn create_dummy_block() -> PreValidatedBlock {
            PreValidatedBlock {
                header: create_dummy_unverified_header(),
                unverified_block_hash: None,
                unverified_block_number: Some(1),
                unverified_global_state_root: Some(felt!("0xa")),
                commitments: create_dummy_commitments(),
                transactions: vec![],
                receipts: vec![],
                state_diff: StateDiff::default(),
                converted_classes: Default::default(),
            }
        }

        /// Test successful trie update.
        ///
        /// Verifies that:
        /// 1. The function correctly updates the tries with the given state diff.
        /// 2. It returns the expected global state root.
        #[tokio::test]
        async fn test_update_tries_success() {
            let backend = setup_test_backend().await;
            let mut block = create_dummy_block();
            block.unverified_global_state_root =
                Some(felt!("0x738e796f750b21ddb3ce528ca88f7e35fad580768bd58571995b19a6809bb4a"));
            block.state_diff = StateDiff {
                deployed_contracts: vec![(DeployedContractItem { address: felt!("0x1"), class_hash: felt!("0x1") })],
                storage_diffs: vec![
                    (ContractStorageDiffItem {
                        address: felt!("0x1"),
                        storage_entries: vec![(StorageEntry { key: felt!("0x1"), value: felt!("0x1") })],
                    }),
                ],
                ..Default::default()
            };
            let validation = BlockValidationContext {
                chain_id: ChainId::Mainnet,
                ignore_block_order: false,
                trust_global_tries: false,
                trust_transaction_hashes: false,
                trust_class_hashes: false,
            };

            let result = update_tries(&backend, &block, &validation, 1).expect("Update tries should succeed");
            assert_eq!(
                result,
                felt!("0x738e796f750b21ddb3ce528ca88f7e35fad580768bd58571995b19a6809bb4a"),
                "Update tries should succeed"
            );
        }

        /// Test trie update with trust_global_tries flag.
        ///
        /// Verifies that:
        /// 1. When trust_global_tries is true, the function returns the unverified global state root.
        #[tokio::test]
        async fn test_update_tries_trust_global_tries() {
            let backend = setup_test_backend().await;
            let block = create_dummy_block();
            let validation = BlockValidationContext {
                chain_id: ChainId::Mainnet,
                ignore_block_order: false,
                trust_global_tries: true,
                trust_transaction_hashes: false,
                trust_class_hashes: false,
            };

            let result = update_tries(&backend, &block, &validation, 1).expect("Update tries should succeed");
            assert_eq!(result, felt!("0xa"), "Should return the unverified global state root");
        }

        /// Test error handling for missing global state root.
        ///
        /// Verifies that:
        /// 1. The function returns an Internal error when the global state root is missing.
        #[tokio::test]
        async fn test_update_tries_missing_global_state_root() {
            let backend = setup_test_backend().await;
            let mut block = create_dummy_block();
            block.unverified_global_state_root = None;
            let validation = BlockValidationContext {
                chain_id: ChainId::Mainnet,
                ignore_block_order: false,
                trust_global_tries: true,
                trust_transaction_hashes: false,
                trust_class_hashes: false,
            };

            let result = update_tries(&backend, &block, &validation, 1);
            assert!(matches!(result.unwrap_err(), BlockImportError::Internal(_)));
        }

        /// Test error handling for mismatched global state root.
        ///
        /// Verifies that:
        /// 1. The function returns a GlobalStateRoot error when the calculated root doesn't match the expected one.
        #[tokio::test]
        async fn test_update_tries_mismatch_global_state_root() {
            let backend = setup_test_backend().await;
            let mut block = create_dummy_block();
            block.unverified_global_state_root = Some(felt!("0xb")); // Different from the calculated root
            let validation = BlockValidationContext {
                chain_id: ChainId::Mainnet,
                ignore_block_order: false,
                trust_global_tries: false,
                trust_transaction_hashes: false,
                trust_class_hashes: false,
            };

            let result = update_tries(&backend, &block, &validation, 1);
            assert!(matches!(result.unwrap_err(), BlockImportError::GlobalStateRoot { .. }));
        }

        /// Test trie update with empty state diff.
        ///
        /// Verifies that:
        /// 1. The function correctly handles an empty state diff.
        /// 2. It returns the expected global state root (0x0 in this case).
        #[tokio::test]
        async fn test_update_tries_empty_state_diff() {
            let backend = setup_test_backend().await;
            let mut block = create_dummy_block(); // StateDiff is already empty by default
            block.unverified_global_state_root = Some(felt!("0x0"));
            let validation = BlockValidationContext {
                chain_id: ChainId::Mainnet,
                ignore_block_order: false,
                trust_global_tries: false,
                trust_transaction_hashes: false,
                trust_class_hashes: false,
            };

            let result = update_tries(&backend, &block, &validation, 1).expect("Update tries should succeed");
            assert_eq!(result, felt!("0x0"), "Update tries should succeed with empty state diff");
        }
    }

    mod block_hash_tests {
        use super::*;
        /// Test successful block hash calculation.
        ///
        /// Verifies that:
        /// 1. The function correctly calculates the block hash.
        /// 2. The returned header contains the expected block number.
        #[test]
        fn test_block_hash_success() {
            let block = PreValidatedBlock {
                header: create_dummy_unverified_header(),
                unverified_block_hash: None,
                unverified_block_number: Some(1),
                unverified_global_state_root: Some(felt!("0xa")),
                commitments: create_dummy_commitments(),
                transactions: vec![],
                receipts: vec![],
                state_diff: StateDiff::default(),
                converted_classes: Default::default(),
            };

            let validation = BlockValidationContext {
                chain_id: ChainId::Other("something".to_string()),
                ignore_block_order: false,
                trust_global_tries: false,
                trust_transaction_hashes: false,
                trust_class_hashes: false,
            };

            let block_number = 1;
            let parent_block_hash = felt!("0x1");
            let global_state_root = felt!("0xa");

            let result = block_hash(&block, &validation, block_number, parent_block_hash, global_state_root).unwrap();

            assert_eq!(
                result.0,
                felt!("0x271814f105da644661d0ef938cfccfd66d3e3585683fbcbee339db3d29c4574"),
                "Block hash should match"
            );
            assert_eq!(result.1.block_number, 1, "Block number should be 1");
        }

        /// Test error handling for mismatched block hash.
        ///
        /// Verifies that:
        /// 1. The function returns a BlockHash error when the calculated hash doesn't match the expected one.
        #[test]
        fn test_block_hash_mismatch() {
            let block = PreValidatedBlock {
                header: create_dummy_unverified_header(),
                unverified_block_hash: Some(felt!("0xdeadbeef")),
                unverified_block_number: Some(1),
                unverified_global_state_root: Some(felt!("0xa")),
                commitments: create_dummy_commitments(),
                transactions: vec![],
                receipts: vec![],
                state_diff: StateDiff::default(),
                converted_classes: Default::default(),
            };

            let validation = BlockValidationContext {
                chain_id: ChainId::Other("something".to_string()),
                ignore_block_order: false,
                trust_global_tries: false,
                trust_transaction_hashes: false,
                trust_class_hashes: false,
            };

            let block_number = 1;
            let parent_block_hash = felt!("0x1");
            let global_state_root = felt!("0xa");

            let result = block_hash(&block, &validation, block_number, parent_block_hash, global_state_root);

            assert!(matches!(result.unwrap_err(), BlockImportError::BlockHash { .. }));
        }

        /// Test special trusted case for block hash calculation.
        ///
        /// Verifies that:
        /// 1. For specific block numbers on mainnet, the function uses the provided block hash.
        ///
        /// Note: This test should be updated/removed when the block hash calculation logic is updated.
        #[test]
        fn test_block_hash_special_trusted_case() {
            let block = PreValidatedBlock {
                header: create_dummy_unverified_header(),
                unverified_block_hash: Some(felt!("0xdeadbeef")),
                unverified_block_number: Some(1466),
                unverified_global_state_root: Some(felt!("0xa")),
                commitments: create_dummy_commitments(),
                transactions: vec![],
                receipts: vec![],
                state_diff: StateDiff::default(),
                converted_classes: Default::default(),
            };

            let validation = BlockValidationContext {
                chain_id: ChainId::Mainnet,
                ignore_block_order: false,
                trust_global_tries: false,
                trust_transaction_hashes: false,
                trust_class_hashes: false,
            };

            let block_number = 1466;
            let parent_block_hash = felt!("0x1");
            let global_state_root = felt!("0xa");

            let result = block_hash(&block, &validation, block_number, parent_block_hash, global_state_root).unwrap();

            assert_eq!(result.0, felt!("0xdeadbeef"), "Should use the provided block hash for special case");
        }
    }

    mod verify_apply_inner_tests {
        use super::*;

        /// Creates a test PreValidatedBlock for verify_apply_inner tests.
        ///
        /// This function generates a PreValidatedBlock with specific values
        /// tailored for testing the verify_apply_inner function.
        fn create_test_block() -> PreValidatedBlock {
            PreValidatedBlock {
                header: create_dummy_unverified_header(),
                unverified_block_hash: None,
                unverified_block_number: Some(1),
                unverified_global_state_root: Some(felt!("0x0")),
                commitments: create_dummy_commitments(),
                transactions: vec![],
                receipts: vec![],
                state_diff: StateDiff::default(),
                converted_classes: Default::default(),
            }
        }

        /// Test successful block verification and storage.
        ///
        /// Verifies that:
        /// 1. The function correctly verifies and stores a new block.
        /// 2. The latest block number is updated after successful storage.
        #[tokio::test]
        async fn test_verify_apply_inner_success_stores_block() {
            let backend = setup_test_backend().await;
            let mut header = create_dummy_header();
            header.block_number = 0;
            let pending_block = finalized_block_zero(header);
            backend.store_block(pending_block.clone(), finalized_state_diff_zero(), vec![]).unwrap();

            assert_eq!(backend.get_latest_block_n().unwrap(), Some(0));

            let mut block = create_test_block();
            block.header.parent_block_hash = Some(felt!("0x12345"));
            let validation = create_validation_context(false);

            let _result = verify_apply_inner(&backend, block.clone(), validation.clone());

            assert_eq!(backend.get_latest_block_n().unwrap(), Some(1));
        }

        /// Test error handling during block verification.
        ///
        /// Verifies that:
        /// 1. The function returns an error for invalid block data (e.g., mismatched block number).
        /// 2. The latest block number remains unchanged when an error occurs.
        #[tokio::test]
        async fn test_verify_apply_inner_error_does_not_store_block() {
            let backend = setup_test_backend().await;
            let mut header = create_dummy_header();
            header.block_number = 0;
            let pending_block = finalized_block_zero(header);
            backend.store_block(pending_block.clone(), finalized_state_diff_zero(), vec![]).unwrap();

            assert_eq!(backend.get_latest_block_n().unwrap(), Some(0));

            let mut block = create_test_block();
            block.header.parent_block_hash = Some(felt!("0x12345"));
            block.unverified_block_number = Some(2); // Mismatch to trigger an error

            let validation = create_validation_context(false);

            let result = verify_apply_inner(&backend, block.clone(), validation);

            assert!(matches!(result.unwrap_err(), BlockImportError::LatestBlockN { .. }));
            assert_eq!(backend.get_latest_block_n().unwrap(), Some(0));
        }
    }
}
