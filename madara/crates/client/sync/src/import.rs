use anyhow::Context;
use mc_db::{db_block_id::RawDbBlockId, MadaraBackend, MadaraStorageError};
use mp_block::{
    commitments::{compute_event_commitment, compute_receipt_commitment, compute_transaction_commitment},
    BlockHeaderWithSignatures, Header, PendingFullBlock, TransactionWithReceipt,
};
use mp_chain_config::StarknetVersion;
use mp_class::{
    class_hash::ComputeClassHashError, compile::ClassCompilationError, ClassInfo, ClassInfoWithHash, ClassType,
    ConvertedClass, LegacyClassInfo, LegacyConvertedClass, SierraClassInfo, SierraConvertedClass,
};
use mp_convert::ToFelt;
use mp_receipt::EventWithTransactionHash;
use mp_state_update::{DeclaredClassCompiledClass, StateDiff};
use mp_utils::rayon::{global_spawn_rayon_task, RayonPool};
use rayon::iter::{IndexedParallelIterator, IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use starknet_api::core::ChainId;
use starknet_core::types::Felt;
use std::{borrow::Cow, collections::HashMap, ops::Range, sync::Arc};

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct BlockValidationConfig {
    /// Trust class hashes.
    pub trust_class_hashes: bool,
    /// Ignore the order of the blocks to allow starting at some height.
    pub trust_parent_hash: bool,

    /// For testing purposes, do not check anything.
    pub no_check: bool,

    /// Save pre-v0.13.2 commitments.
    pub pre_v0_13_2_commitments: bool,
}

impl BlockValidationConfig {
    pub fn trust_parent_hash(self, trust_parent_hash: bool) -> Self {
        Self { trust_parent_hash, ..self }
    }
    pub fn all_verifications_disabled(self, no_check: bool) -> Self {
        Self { no_check, ..self }
    }
    pub fn pre_v0_13_2_commitments(self, pre_v0_13_2_commitments: bool) -> Self {
        Self { pre_v0_13_2_commitments, ..self }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BlockImportError {
    #[error("Transaction count mismatch: expected {expected}, got {got}")]
    TransactionCount { got: u64, expected: u64 },
    #[error("Transaction commitment mismatch: expected {expected:#x}, got {got:#x}")]
    TransactionCommitment { got: Felt, expected: Felt },

    #[error("Event count mismatch: expected {expected}, got {got}")]
    EventCount { got: u64, expected: u64 },
    #[error("Event commitment mismatch: expected {expected:#x}, got {got:#x}")]
    EventCommitment { got: Felt, expected: Felt },

    #[error("State diff length mismatch: expected {expected}, got {got}")]
    StateDiffLength { got: u64, expected: u64 },
    #[error("State diff commitment mismatch: expected {expected:#x}, got {got:#x}")]
    StateDiffCommitment { got: Felt, expected: Felt },

    #[error("Receipt commitment mismatch: expected {expected:#x}, got {got:#x}")]
    ReceiptCommitment { got: Felt, expected: Felt },

    #[error("Unexpected class: {class_hash:#x}")]
    UnexpectedClass { class_hash: Felt },
    #[error("Class type mismatch for class hash {class_hash:#x}: expected {expected}, got {got}")]
    ClassType { class_hash: Felt, got: ClassType, expected: ClassType },
    #[error("Class hash mismatch: expected {expected:#x}, got {got:#x}")]
    ClassHash { got: Felt, expected: Felt },
    #[error("Class count mismatch: expected {expected}, got {got}")]
    ClassCount { got: u64, expected: u64 },
    #[error("Compiled class hash mismatch for class hash {class_hash:#x}: expected {expected:#x}, got {got:#x}")]
    CompiledClassHash { class_hash: Felt, got: Felt, expected: Felt },
    #[error("Class with hash {class_hash:#x} failed to compile: {error}")]
    CompilationClassError { class_hash: Felt, error: ClassCompilationError },
    #[error("Failed to compute class hash {class_hash:#x}: {error}")]
    ComputeClassHash { class_hash: Felt, error: ComputeClassHashError },

    #[error("Block number mismatch: expected {expected:#x}, got {got:#x}")]
    BlockNumber { got: u64, expected: u64 },

    #[error("Global state root mismatch: expected {expected:#x}, got {got:#x}")]
    GlobalStateRoot { got: Felt, expected: Felt },
    /// Internal error, see [`BlockImportError::is_internal`].
    #[error("Internal database error while {context}: {error:#}")]
    InternalDb { context: Cow<'static, str>, error: MadaraStorageError },
    /// Internal error, see [`BlockImportError::is_internal`].
    #[error("Internal error: {0}")]
    Internal(#[from] anyhow::Error),
}
impl BlockImportError {
    /// Unrecoverable errors.
    pub fn is_internal(&self) -> bool {
        matches!(self, BlockImportError::InternalDb { .. } | BlockImportError::Internal(_))
    }
}

/// Shared verification & saving logic between gateway and (yet-to-be-merged) p2p.
#[derive(Clone)]
pub struct BlockImporter {
    db: Arc<MadaraBackend>,
    config: BlockValidationConfig,
    rayon_pool: Arc<RayonPool>,
}

impl BlockImporter {
    pub fn new(db: Arc<MadaraBackend>, config: BlockValidationConfig) -> BlockImporter {
        Self { db, config, rayon_pool: Arc::new(RayonPool::new()) }
    }

    pub async fn run_in_rayon_pool<F, R>(&self, func: F) -> R
    where
        F: FnOnce(BlockImporterCtx) -> R + Send + 'static,
        R: Send + 'static,
    {
        let ctx = self.ctx();
        self.rayon_pool.spawn_rayon_task(move || func(ctx)).await
    }

    /// This is only used for apply global trie. It is applied from a sequential step,
    /// and thus we want to avoid taking up a permit, to avoid deadlocks.
    pub async fn run_in_rayon_pool_global<F, R>(&self, func: F) -> R
    where
        F: FnOnce(BlockImporterCtx) -> R + Send + 'static,
        R: Send + 'static,
    {
        let ctx = self.ctx();
        global_spawn_rayon_task(move || func(ctx)).await
    }

    fn ctx(&self) -> BlockImporterCtx {
        BlockImporterCtx { db: self.db.clone(), config: self.config.clone() }
    }
}

pub struct BlockImporterCtx {
    db: Arc<MadaraBackend>,
    config: BlockValidationConfig,
}
impl BlockImporterCtx {
    // Pending block

    pub fn save_pending_block(&self, block: PendingFullBlock) -> Result<(), BlockImportError> {
        self.db
            .store_pending_block(block)
            .map_err(|error| BlockImportError::InternalDb { error, context: "Storing pending block".into() })?;
        Ok(())
    }

    pub fn save_pending_classes(&self, classes: Vec<ConvertedClass>) -> Result<(), BlockImportError> {
        self.db
            .class_db_store_pending(&classes)
            .map_err(|error| BlockImportError::InternalDb { error, context: "Storing pending classes".into() })?;
        Ok(())
    }

    // HEADERS

    pub fn verify_header(
        &self,
        block_n: u64,
        signed_header: &BlockHeaderWithSignatures,
    ) -> Result<(), BlockImportError> {
        // TODO

        // TODO: verify signatures

        // verify block_number
        if !self.config.no_check && block_n != signed_header.header.block_number {
            return Err(BlockImportError::BlockNumber { expected: block_n, got: signed_header.header.block_number });
        }

        // verify block_hash
        // TODO: pre_v0_13_2_override
        let _block_hash = signed_header
            .header
            .compute_hash(self.db.chain_config().chain_id.to_felt(), /* pre_v0_13_2_override */ true);
        // if signed_header.block_hash != block_hash {
        //     return Err(P2pError::peer_error(format!(
        //         "Mismatched block_hash: {:#x}, expected {:#x}",
        //         signed_header.block_hash, block_hash
        //     )));
        // }

        Ok(())
    }

    pub fn save_header(&self, block_n: u64, signed_header: BlockHeaderWithSignatures) -> Result<(), BlockImportError> {
        self.db.store_block_header(signed_header).map_err(|error| BlockImportError::InternalDb {
            error,
            context: format!("Storing block header for {block_n}").into(),
        })?;
        Ok(())
    }

    // TRANSACTIONS & RECEIPTS

    /// Called in a rayon-pool context.
    /// Returns the transactions and receipt commitment.
    pub fn verify_transactions(
        &self,
        _block_n: u64,
        transactions: &[TransactionWithReceipt],
        check_against: &Header,
        allow_pre_v0_13_2: bool,
    ) -> Result<(Felt, Felt), BlockImportError> {
        // Override pre-v0.13.2 transaction hash computation
        let starknet_version = StarknetVersion::max(check_against.protocol_version, StarknetVersion::V0_13_2);
        let is_pre_v0_13_2_special_case =
            allow_pre_v0_13_2 && check_against.protocol_version < StarknetVersion::V0_13_2;

        // Verify transaction hashes. Also compute the (hash with signature, receipt hash).
        let tx_hashes_with_signature_and_receipt_hashes: Vec<_> = transactions
            .par_iter()
            .enumerate()
            .map(|(_index, tx)| {
                let got = tx.transaction.compute_hash(
                    self.db.chain_config().chain_id.to_felt(),
                    starknet_version,
                    /* is_query */ false,
                );
                Ok((tx.transaction.compute_hash_with_signature(got, starknet_version), tx.receipt.compute_hash()))
            })
            .collect::<Result<_, BlockImportError>>()?;

        // Verify transaction count (we want to check it when the block does not come from p2p).
        let expected = check_against.transaction_count;
        let got = transactions.len() as _;
        if !self.config.no_check && expected != got {
            return Err(BlockImportError::TransactionCount { got, expected });
        }

        // Verify transaction commitment.
        let expected = check_against.transaction_commitment;
        let transaction_commitment = compute_transaction_commitment(
            tx_hashes_with_signature_and_receipt_hashes.iter().map(|(fst, _)| *fst),
            starknet_version,
        );
        if !self.config.no_check && !is_pre_v0_13_2_special_case && expected != transaction_commitment {
            return Err(BlockImportError::TransactionCommitment { got: transaction_commitment, expected });
        }

        // Verify receipt commitment.
        let expected = check_against.receipt_commitment.unwrap_or_default();
        let receipt_commitment = compute_receipt_commitment(
            tx_hashes_with_signature_and_receipt_hashes.iter().map(|(_, snd)| *snd),
            starknet_version,
        );
        if !self.config.no_check && !is_pre_v0_13_2_special_case && expected != receipt_commitment {
            return Err(BlockImportError::ReceiptCommitment { got: receipt_commitment, expected });
        }

        Ok((transaction_commitment, receipt_commitment))
    }

    /// Called in a rayon-pool context.
    pub fn save_transactions(
        &self,
        block_n: u64,
        transactions: Vec<TransactionWithReceipt>,
    ) -> Result<(), BlockImportError> {
        tracing::debug!("Storing transactions for {block_n:?}");
        self.db.store_transactions(block_n, transactions).map_err(|error| BlockImportError::InternalDb {
            error,
            context: format!("Storing transactions for {block_n}").into(),
        })?;
        Ok(())
    }

    // CLASSES

    /// Called in a rayon-pool context.
    pub fn verify_compile_classes(
        &self,
        block_n: Option<u64>,
        declared_classes: Vec<ClassInfoWithHash>,
        check_against: &HashMap<Felt, DeclaredClassCompiledClass>,
    ) -> Result<Vec<ConvertedClass>, BlockImportError> {
        if check_against.len() != declared_classes.len() {
            return Err(BlockImportError::ClassCount {
                got: declared_classes.len() as _,
                expected: check_against.len() as _,
            });
        }
        let classes = declared_classes
            .into_par_iter()
            .map(|class| self.verify_compile_class(block_n, class, check_against))
            .collect::<Result<_, _>>()?;
        Ok(classes)
    }

    /// Called in a rayon-pool context.
    fn verify_compile_class(
        &self,
        block_n: Option<u64>,
        class: ClassInfoWithHash,
        check_against: &HashMap<Felt, DeclaredClassCompiledClass>,
    ) -> Result<ConvertedClass, BlockImportError> {
        let class_hash = class.class_hash;

        let check_against = *check_against.get(&class_hash).ok_or(BlockImportError::UnexpectedClass { class_hash })?;

        match class.class_info {
            ClassInfo::Sierra(sierra) => {
                tracing::trace!("Converting class with hash {:#x}", class_hash);

                let DeclaredClassCompiledClass::Sierra(expected) = check_against else {
                    return Err(BlockImportError::ClassType {
                        class_hash,
                        got: ClassType::Legacy,
                        expected: ClassType::Sierra,
                    });
                };
                if !self.config.no_check && sierra.compiled_class_hash != expected {
                    return Err(BlockImportError::CompiledClassHash {
                        class_hash,
                        got: sierra.compiled_class_hash,
                        expected,
                    });
                }

                // Verify class hash
                if !self.config.no_check && !self.config.trust_class_hashes {
                    let expected = sierra
                        .contract_class
                        .compute_class_hash()
                        .map_err(|error| BlockImportError::ComputeClassHash { class_hash, error })?;
                    if !self.config.no_check && class_hash != expected {
                        return Err(BlockImportError::ClassHash { got: class_hash, expected });
                    }
                }

                // Compile
                let (compiled_class_hash, compiled_class) = sierra
                    .contract_class
                    .compile_to_casm()
                    .map_err(|e| BlockImportError::CompilationClassError { class_hash, error: e })?;

                // Verify compiled class hash
                if !self.config.no_check && compiled_class_hash != sierra.compiled_class_hash {
                    return Err(BlockImportError::CompiledClassHash {
                        class_hash,
                        got: sierra.compiled_class_hash,
                        expected: compiled_class_hash,
                    });
                }
                Ok(ConvertedClass::Sierra(SierraConvertedClass {
                    class_hash,
                    info: SierraClassInfo { contract_class: sierra.contract_class, compiled_class_hash },
                    compiled: Arc::new(compiled_class),
                }))
            }
            ClassInfo::Legacy(legacy) => {
                tracing::trace!("Converting legacy class with hash {:#x}", class_hash);

                if !self.config.no_check && check_against != DeclaredClassCompiledClass::Legacy {
                    return Err(BlockImportError::ClassType {
                        class_hash,
                        got: ClassType::Sierra,
                        expected: ClassType::Legacy,
                    });
                }

                // Verify class hash
                if !self.config.trust_class_hashes {
                    let mut expected = legacy
                        .contract_class
                        .compute_class_hash()
                        .map_err(|e| BlockImportError::ComputeClassHash { class_hash, error: e })?;

                    if let Some(block_n) = block_n {
                        if self.db.chain_config().chain_id == ChainId::Mainnet {
                            // We do not actually implement class hash verification for some cairo 0 classes.
                            // See [`mp_class::mainnet_legacy_class_hashes`] for more information about this; but this
                            // only applies to a few classes on mainnet in total. We have decided to just hardcode them.
                            expected = mp_class::mainnet_legacy_class_hashes::get_real_class_hash(block_n, expected)
                        }
                    }

                    if !self.config.no_check && class_hash != expected {
                        // tracing::info!("Mismatched class_hash={class_hash:#x} expected={expected:#x} block_n={block_n:?}");
                        return Err(BlockImportError::ClassHash { got: class_hash, expected });
                    }
                }

                Ok(ConvertedClass::Legacy(LegacyConvertedClass {
                    class_hash,
                    info: LegacyClassInfo { contract_class: legacy.contract_class },
                }))
            }
        }
    }

    /// Called in a rayon-pool context.
    pub fn save_classes(&self, block_n: u64, classes: Vec<ConvertedClass>) -> Result<(), BlockImportError> {
        self.db.class_db_store_block(block_n, &classes).map_err(|error| BlockImportError::InternalDb {
            error,
            context: format!("Storing classes for {block_n}").into(),
        })?;
        Ok(())
    }

    // STATE DIFF

    /// Called in a rayon-pool context.
    /// Returns the state diff commitment.
    pub fn verify_state_diff(
        &self,
        _block_n: u64,
        state_diff: &StateDiff,
        check_against: &Header,
        allow_pre_v0_13_2: bool,
    ) -> Result<Felt, BlockImportError> {
        let is_pre_v0_13_2_special_case =
            allow_pre_v0_13_2 && check_against.protocol_version < StarknetVersion::V0_13_2;

        // Verify state diff length (we want to check it when the block does not come from p2p).
        let expected = check_against.state_diff_length.unwrap_or_default();
        let got = state_diff.len() as _;
        if !self.config.no_check && expected != got {
            return Err(BlockImportError::StateDiffLength { got, expected });
        }

        // Verify state diff commitment.
        let expected = check_against.state_diff_commitment.unwrap_or_default();
        let got = state_diff.compute_hash();
        if !self.config.no_check && !is_pre_v0_13_2_special_case && expected != got {
            return Err(BlockImportError::StateDiffCommitment { got, expected });
        }
        Ok(got)
    }

    /// Called in a rayon-pool context.
    pub fn save_state_diff(&self, block_n: u64, state_diff: StateDiff) -> Result<(), BlockImportError> {
        self.db.store_state_diff(block_n, state_diff).map_err(|error| BlockImportError::InternalDb {
            error,
            context: format!("Storing state_diff for {block_n}").into(),
        })?;
        Ok(())
    }

    // EVENTS

    /// Called in a rayon-pool context.
    /// Returns the event commitment.
    pub fn verify_events(
        &self,
        _block_n: u64,
        events: &[EventWithTransactionHash],
        check_against: &Header,
        allow_pre_v0_13_2: bool,
    ) -> Result<Felt, BlockImportError> {
        // Override pre-v0.13.2 transaction hash computation
        let starknet_version = StarknetVersion::max(check_against.protocol_version, StarknetVersion::V0_13_2);
        let is_pre_v0_13_2_special_case =
            allow_pre_v0_13_2 && check_against.protocol_version < StarknetVersion::V0_13_2;

        let event_hashes: Vec<_> =
            events.par_iter().map(|ev| ev.event.compute_hash(ev.transaction_hash, starknet_version)).collect();

        // Verify event count (we want to check it when the block does not come from p2p).
        let expected = check_against.event_count;
        let got = events.len() as _;
        if !self.config.no_check && expected != got {
            return Err(BlockImportError::EventCount { got, expected });
        }

        // Verify events commitment.
        let expected = check_against.event_commitment;
        let got = compute_event_commitment(event_hashes, starknet_version);
        if !self.config.no_check && !is_pre_v0_13_2_special_case && expected != got {
            return Err(BlockImportError::EventCommitment { got, expected });
        }

        Ok(got)
    }

    /// Called in a rayon-pool context.
    pub fn save_events(&self, block_n: u64, events: Vec<EventWithTransactionHash>) -> Result<(), BlockImportError> {
        self.db.store_events(block_n, events).map_err(|error| BlockImportError::InternalDb {
            error,
            context: format!("Storing events for {block_n}").into(),
        })?;
        Ok(())
    }

    // GLOBAL TRIE

    pub fn apply_to_global_trie(
        &self,
        block_range: Range<u64>,
        state_diffs: Vec<StateDiff>,
    ) -> Result<(), BlockImportError> {
        if block_range.is_empty() {
            return Ok(());
        }
        let got = self.db.apply_to_global_trie(block_range.start, state_diffs.iter()).map_err(|error| {
            BlockImportError::InternalDb { error, context: "Applying state diff to global trie".into() }
        })?;

        // Sanity check: verify state root.
        if !self.config.no_check {
            let block_n = block_range.last().expect("Range checked for empty earlier.");
            let expected = self
                .db
                .get_block_info(&RawDbBlockId::Number(block_n))
                .map_err(|error| BlockImportError::InternalDb {
                    error,
                    context: format!("Cannot find block info for block #{block_n}").into(),
                })?
                .context("Block header cannot be found")?
                .as_nonpending_owned()
                .context("Block is pending")?
                .header
                .global_state_root;

            if expected != got {
                return Err(BlockImportError::GlobalStateRoot { got, expected });
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{BlockImportError, BlockImporter, BlockValidationConfig};
    use mc_db::MadaraBackend;
    use mp_block::{BlockHeaderWithSignatures, Header};
    use mp_chain_config::ChainConfig;
    use mp_state_update::{ContractStorageDiffItem, DeployedContractItem, StateDiff, StorageEntry};
    use rstest::*;
    use starknet_api::felt;
    use starknet_core::types::Felt;
    use std::sync::Arc;

    /// Test cases for the `update_tries` function.
    ///
    /// This test uses `rstest` to parameterize different scenarios for updating the tries.
    /// It verifies that the function correctly handles various input combinations and
    /// produces the expected results or errors.
    #[rstest]
    #[case::success(
            // A non-zero global state root
            felt!("0x738e796f750b21ddb3ce528ca88f7e35fad580768bd58571995b19a6809bb4a"),
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
            Ok(())
        )]
    #[case::mismatch_global_state_root(
            felt!("0xb"), // A non-zero global state root
            StateDiff::default(), // Empty state diff
            // Expected result: a GlobalStateRoot error due to mismatch
            Err(BlockImportError::GlobalStateRoot { expected: felt!("0xb"), got: felt!("0x0") })
        )]
    #[case::empty_state_diff(
            felt!("0x0"), // Zero global state root
            StateDiff::default(), // Empty state diff
            Ok(())
        )]
    #[tokio::test]
    async fn test_update_tries(
        #[case] unverified_global_state_root: Felt,
        #[case] state_diff: StateDiff,
        #[case] expected_result: Result<(), BlockImportError>,
    ) {
        // GIVEN: We have a test backend and a block with specified parameters
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        backend
            .store_block_header(BlockHeaderWithSignatures {
                block_hash: felt!("0x123123"),
                consensus_signatures: vec![],
                header: Header {
                    global_state_root: unverified_global_state_root,
                    block_number: 0,
                    ..Default::default()
                },
            })
            .unwrap();

        // AND: We have a validation context with specified trust_global_tries
        let validation = BlockValidationConfig::default();
        let importer = BlockImporter::new(backend, validation);

        // WHEN: We call update_tries with these parameters
        let result = importer.ctx().apply_to_global_trie(0..1, vec![state_diff]);

        assert_eq!(expected_result.map_err(|e| format!("{e:#}")), result.map_err(|e| format!("{e:#}")),)
    }
}
