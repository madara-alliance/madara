use mc_db::{db_block_id::DbBlockId, MadaraBackend, MadaraStorageError};
use mp_block::{
    commitments::{compute_event_commitment, compute_receipt_commitment, compute_transaction_commitment},
    BlockHeaderWithSignatures, Header, TransactionWithReceipt,
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
use starknet_core::types::Felt;
use std::{borrow::Cow, collections::HashMap, ops::Range, sync::Arc};

use crate::update_global_trie;

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct BlockValidationConfig {
    /// Trust class hashes.
    pub trust_class_hashes: bool,
    // /// Ignore the order of the blocks to allow starting at some height.
    // pub trust_parent_hash: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum BlockImportError {
    // #[error("Transaction hash mismatch for index #{index}: expected {expected:#x}, got {got:#x}")]
    // TransactionHash { index: usize, got: Felt, expected: Felt },
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

    // #[error("Block hash mismatch: expected {expected:#x}, got {got:#x}")]
    // BlockHash { got: Felt, expected: Felt },
    #[error("Block number mismatch: expected {expected:#x}, got {got:#x}")]
    BlockNumber { got: u64, expected: u64 },

    // #[error("Block order mismatch: database expects to import block #{expected}, trying to import #{got}. To import a block out of order, use the `ignore_block_order` flag.")]
    // LatestBlockN { expected: u64, got: u64 },
    // #[error("Parent hash mismatch: expected {expected:#x}, got {got:#x}")]
    // ParentHash { got: Felt, expected: Felt },
    #[error("Global state root mismatch: expected {expected:#x}, got {got:#x}")]
    GlobalStateRoot { got: Felt, expected: Felt },
    /// Internal error, see [`BlockImportError::is_internal`].
    #[error("Internal database error while {context}: {error:#}")]
    InternalDb { context: Cow<'static, str>, error: MadaraStorageError },
    /// Internal error, see [`BlockImportError::is_internal`].
    #[error("Internal error: {0}")]
    Internal(Cow<'static, str>),
}
impl BlockImportError {
    /// Unrecoverable errors.
    pub fn is_internal(&self) -> bool {
        matches!(self, BlockImportError::InternalDb { .. } | BlockImportError::Internal(_))
    }
}

/// Shared verification & saving logic between gateway and p2p.
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

    /// Does class compilation.
    pub async fn run_in_rayon_pool<F, R>(&self, func: F) -> R
    where
        F: FnOnce(&BlockImporter) -> R + Send + 'static,
        R: Send + 'static,
    {
        let this = self.clone();
        self.rayon_pool.spawn_rayon_task(move || func(&this)).await
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
        if block_n != signed_header.header.block_number {
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
                // For pre-v0.13.2, our tx hash is only used for commitment computation.
                // let expected = tx.receipt.transaction_hash();
                // // if expected != got {
                // //     return Err(BlockImportError::TransactionHash { index, got, expected });
                // // }
                Ok((tx.transaction.compute_hash_with_signature(got, starknet_version), tx.receipt.compute_hash()))
            })
            .collect::<Result<_, BlockImportError>>()?;

        // Verify transaction count (we want to check it when the block does not come from p2p).
        let expected = check_against.transaction_count;
        let got = transactions.len() as _;
        if expected != got {
            return Err(BlockImportError::TransactionCount { got, expected });
        }

        // Verify transaction commitment.
        let expected = check_against.transaction_commitment;
        let transaction_commitment = compute_transaction_commitment(
            tx_hashes_with_signature_and_receipt_hashes.iter().map(|(fst, _)| *fst),
            starknet_version,
        );
        if !is_pre_v0_13_2_special_case && expected != transaction_commitment {
            return Err(BlockImportError::TransactionCommitment { got: transaction_commitment, expected });
        }

        // Verify receipt commitment.
        let expected = check_against.receipt_commitment.unwrap_or_default();
        let receipt_commitment = compute_receipt_commitment(
            tx_hashes_with_signature_and_receipt_hashes.iter().map(|(_, snd)| *snd),
            starknet_version,
        );
        if !is_pre_v0_13_2_special_case && expected != receipt_commitment {
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
        _block_n: u64,
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
            .map(|class| self.verify_compile_class(class, check_against))
            .collect::<Result<_, _>>()?;
        Ok(classes)
    }

    /// Called in a rayon-pool context.
    fn verify_compile_class(
        &self,
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
                if sierra.compiled_class_hash != expected {
                    return Err(BlockImportError::CompiledClassHash {
                        class_hash,
                        got: sierra.compiled_class_hash,
                        expected,
                    });
                }

                // Verify class hash
                if !self.config.trust_class_hashes {
                    let expected = sierra
                        .contract_class
                        .compute_class_hash()
                        .map_err(|error| BlockImportError::ComputeClassHash { class_hash, error })?;
                    if class_hash != expected {
                        return Err(BlockImportError::ClassHash { got: class_hash, expected });
                    }
                }

                // Compile
                let (compiled_class_hash, compiled_class) = sierra
                    .contract_class
                    .compile_to_casm()
                    .map_err(|e| BlockImportError::CompilationClassError { class_hash, error: e })?;

                // Verify compiled class hash
                if compiled_class_hash != sierra.compiled_class_hash {
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

                if check_against != DeclaredClassCompiledClass::Legacy {
                    return Err(BlockImportError::ClassType {
                        class_hash,
                        got: ClassType::Sierra,
                        expected: ClassType::Legacy,
                    });
                }

                // Verify class hash
                if !self.config.trust_class_hashes {
                    let expected = legacy
                        .contract_class
                        .compute_class_hash()
                        .map_err(|e| BlockImportError::ComputeClassHash { class_hash, error: e })?;

                    if class_hash != expected {
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
        if expected != got {
            return Err(BlockImportError::StateDiffLength { got, expected });
        }

        // Verify state diff commitment.
        let expected = check_against.state_diff_commitment.unwrap_or_default();
        let got = state_diff.compute_hash();
        if !is_pre_v0_13_2_special_case && expected != got {
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
        if expected != got {
            return Err(BlockImportError::EventCount { got, expected });
        }

        // Verify events commitment.
        let expected = check_against.event_commitment;
        let got = compute_event_commitment(event_hashes, starknet_version);
        if !is_pre_v0_13_2_special_case && expected != got {
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

    pub async fn apply_to_global_trie(
        &self,
        block_range: Range<u64>,
        state_diffs: Vec<StateDiff>,
    ) -> anyhow::Result<()> {
        let this = self.clone();
        // do not use the shared permits for a sequential step
        global_spawn_rayon_task(move || this.apply_to_global_trie_inner(block_range, state_diffs)).await?;
        Ok(())
    }

    /// Called in a rayon-pool context.
    fn apply_to_global_trie_inner(
        &self,
        block_range: Range<u64>,
        state_diffs: Vec<StateDiff>,
    ) -> Result<(), BlockImportError> {
        for (block_n, state_diff) in block_range.zip(state_diffs) {
            tracing::debug!("applying state_diff block_n={block_n} {state_diff:#?}");
            let (contract_trie_root, class_trie_root) = rayon::join(
                || {
                    update_global_trie::contracts::contract_trie_root(
                        &self.db,
                        &state_diff.deployed_contracts,
                        &state_diff.replaced_classes,
                        &state_diff.nonces,
                        &state_diff.storage_diffs,
                        block_n,
                    )
                },
                || update_global_trie::classes::class_trie_root(&self.db, &state_diff.declared_classes, block_n),
            );

            let Some(block_info) =
                self.db.get_block_info(&DbBlockId::Number(block_n)).map_err(|error| BlockImportError::InternalDb {
                    error,
                    context: format!("Getting block header for block #{block_n}").into(),
                })?
            else {
                return Err(BlockImportError::Internal(
                    format!("Cannot find block header for block #{block_n}").into(),
                ));
            };

            let Some(block_info) = block_info.as_nonpending() else {
                return Err(BlockImportError::Internal(format!("Block at #{block_n} is pending").into()));
            };

            // Check the global_state_root

            let expected = block_info.header.global_state_root;
            let got = update_global_trie::calculate_state_root(
                contract_trie_root.map_err(|error| BlockImportError::InternalDb {
                    error,
                    context: format!("Updating contract trie for block #{block_n}").into(),
                })?,
                class_trie_root.map_err(|error| BlockImportError::InternalDb {
                    error,
                    context: format!("Updating class trie for block #{block_n}").into(),
                })?,
            );

            if expected != got {
                return Err(BlockImportError::GlobalStateRoot { got, expected });
            }
        }
        Ok(())
    }
}
