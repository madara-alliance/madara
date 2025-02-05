use crate::{
    BlockImportError, BlockValidationContext, DeclaredClass, PreValidatedBlock, PreValidatedPendingBlock, RayonPool,
    UnverifiedFullBlock, UnverifiedPendingFullBlock, ValidatedCommitments,
};
use bitvec::vec::BitVec;
use mp_chain_config::StarknetVersion;
use mp_class::{ConvertedClass, LegacyClassInfo, LegacyConvertedClass, SierraClassInfo, SierraConvertedClass};
use mp_convert::ToFelt;
use mp_receipt::TransactionReceipt;
use mp_transactions::Transaction;
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator};
use rayon::prelude::*;
use starknet_api::core::ChainId;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Pedersen, Poseidon, StarkHash};
use std::mem;
use std::sync::Arc;

/// This function wraps the [`pre_validate_inner`] step, which runs on the rayon pool, in a tokio-friendly future.
pub async fn pre_validate(
    pool: &RayonPool,
    block: UnverifiedFullBlock,
    validation: BlockValidationContext,
) -> Result<PreValidatedBlock, BlockImportError> {
    tracing::trace!("spawning pre_validate");
    let res = pool.spawn_rayon_task(move || pre_validate_inner(block, validation)).await;
    tracing::trace!("finished pre_validate");
    res
}

/// See [`pre_validate`].
pub async fn pre_validate_pending(
    pool: &RayonPool,
    block: UnverifiedPendingFullBlock,
    validation: BlockValidationContext,
) -> Result<PreValidatedPendingBlock, BlockImportError> {
    tracing::trace!("spawning pre_validate (pending)");
    let res = pool.spawn_rayon_task(move || pre_validate_pending_inner(block, validation)).await;
    tracing::trace!("finished pre_validate (pending)");
    res
}

/// This runs on the [`rayon`] threadpool.
pub fn pre_validate_inner(
    mut block: UnverifiedFullBlock,
    validation: BlockValidationContext,
) -> Result<PreValidatedBlock, BlockImportError> {
    let classes = mem::take(&mut block.declared_classes);

    // unfortunately this is ugly but rayon::join does not have the fast error short circuiting behavior that
    // collecting into a Result has.
    // little known fact this uses the impl FromIterator for () from std, nice trick
    let (mut commitments, mut converted_classes) = Default::default();
    [
        Box::new(|| {
            commitments = block_commitments(&block, &validation)?;
            Ok(())
        }) as Box<dyn FnOnce() -> Result<(), BlockImportError> + Send>,
        Box::new(|| {
            converted_classes = convert_classes(classes, &validation)?;
            Ok(())
        }),
    ]
    .into_par_iter()
    .map(|f| f())
    .collect::<Result<(), _>>()?;

    converted_classes.extend(block.trusted_converted_classes);

    Ok(PreValidatedBlock {
        header: block.header,
        transactions: block.transactions,
        state_diff: block.state_diff,
        receipts: block.receipts,
        commitments,
        converted_classes,
        unverified_global_state_root: block.commitments.global_state_root,
        unverified_block_hash: block.commitments.block_hash,
        unverified_block_number: block.unverified_block_number,
        visited_segments: block.visited_segments,
    })
}

/// See [`pre_validate_inner`].
pub fn pre_validate_pending_inner(
    mut block: UnverifiedPendingFullBlock,
    validation: BlockValidationContext,
) -> Result<PreValidatedPendingBlock, BlockImportError> {
    let starknet_version = block.header.protocol_version;
    let classes = mem::take(&mut block.declared_classes);

    let converted_classes = convert_classes(classes, &validation)?;
    let _tx_hashes = transaction_hashes(&block.receipts, &block.transactions, starknet_version, &validation)?;

    Ok(PreValidatedPendingBlock {
        header: block.header,
        transactions: block.transactions,
        state_diff: block.state_diff,
        receipts: block.receipts,
        converted_classes,
        visited_segments: block.visited_segments,
    })
}

fn block_commitments(
    block: &UnverifiedFullBlock,
    validation: &BlockValidationContext,
) -> Result<ValidatedCommitments, BlockImportError> {
    let (mut receipt_c, mut state_diff_c, mut transaction_c, mut event_c) = Default::default();
    [
        Box::new(|| {
            receipt_c = receipt_commitment(block, validation)?;
            Ok(())
        }) as Box<dyn FnOnce() -> Result<(), BlockImportError> + Send>,
        Box::new(|| {
            state_diff_c = state_diff_commitment(block, validation)?;
            Ok(())
        }),
        Box::new(|| {
            transaction_c = transaction_commitment(block, validation)?;
            Ok(())
        }),
        Box::new(|| {
            event_c = event_commitment(block, validation)?;
            Ok(())
        }),
    ]
    .into_par_iter()
    .map(|f| f())
    .collect::<Result<(), _>>()?;

    Ok(ValidatedCommitments {
        transaction_count: block.transactions.len() as _,
        transaction_commitment: transaction_c,
        event_count: block.receipts.iter().map(|r| r.events().len()).sum::<usize>() as _,
        event_commitment: event_c,
        state_diff_length: block.state_diff.len() as _,
        state_diff_commitment: state_diff_c,
        receipt_commitment: receipt_c,
    })
}

fn convert_classes(
    declared_classes: Vec<DeclaredClass>,
    validation: &BlockValidationContext,
) -> Result<Vec<ConvertedClass>, BlockImportError> {
    declared_classes.into_par_iter().map(|class| class_conversion(class, validation)).collect()
}

fn class_conversion(
    class: DeclaredClass,
    validation: &BlockValidationContext,
) -> Result<ConvertedClass, BlockImportError> {
    match class {
        DeclaredClass::Sierra(sierra) => {
            tracing::trace!("Converting class with hash {:#x}", sierra.class_hash);
            if !validation.trust_class_hashes {
                let class_hash = sierra
                    .contract_class
                    .compute_class_hash()
                    .map_err(|e| BlockImportError::ComputeClassHash { class_hash: sierra.class_hash, error: e })?;
                if class_hash != sierra.class_hash {
                    return Err(BlockImportError::ClassHash { got: sierra.class_hash, expected: class_hash });
                }
            }
            let (compiled_class_hash, compiled_class) = sierra
                .contract_class
                .compile_to_casm()
                .map_err(|e| BlockImportError::CompilationClassError { class_hash: sierra.class_hash, error: e })?;
            if compiled_class_hash != sierra.compiled_class_hash {
                return Err(BlockImportError::CompiledClassHash {
                    class_hash: sierra.class_hash,
                    got: sierra.compiled_class_hash,
                    expected: compiled_class_hash,
                });
            }
            Ok(ConvertedClass::Sierra(SierraConvertedClass {
                class_hash: sierra.class_hash,
                info: SierraClassInfo { contract_class: Arc::new(sierra.contract_class), compiled_class_hash },
                compiled: Arc::new(compiled_class),
            }))
        }
        DeclaredClass::Legacy(legacy) => {
            tracing::trace!("Converting legacy class with hash {:#x}", legacy.class_hash);
            if !validation.trust_class_hashes {
                let class_hash = legacy
                    .contract_class
                    .compute_class_hash()
                    .map_err(|e| BlockImportError::ComputeClassHash { class_hash: legacy.class_hash, error: e })?;
                if class_hash != legacy.class_hash {
                    // TODO: For now we skip the exceptions for the legacy class hash mismatch
                    tracing::debug!("Class hash mismatch: got {:#x}, expected {:#x}", class_hash, legacy.class_hash,);
                    // return Err(BlockImportError::ClassHash { got: class_hash, expected: legacy.class_hash });
                }
            }
            Ok(ConvertedClass::Legacy(LegacyConvertedClass {
                class_hash: legacy.class_hash,
                info: LegacyClassInfo { contract_class: Arc::new(legacy.contract_class) },
            }))
        }
    }
}

fn transaction_hashes(
    receipts: &[TransactionReceipt],
    transactions: &[Transaction],
    mut starknet_version: StarknetVersion,
    validation: &BlockValidationContext,
) -> Result<Vec<Felt>, BlockImportError> {
    if receipts.len() != transactions.len() {
        return Err(BlockImportError::TransactionEqualReceiptCount {
            receipts: receipts.len(),
            transactions: transactions.len(),
        });
    }

    // compute_v0_13_2_hashes mode
    let compute_v0_13_2_hashes_mode = validation.compute_v0_13_2_hashes && starknet_version < StarknetVersion::V0_13_2;
    if compute_v0_13_2_hashes_mode {
        starknet_version = StarknetVersion::V0_13_2;
    }

    // mismatched block hash is allowed for block 1469 on mainnet
    // this block contains a part of transactions computed with the legacy hash function
    // and the other part with the post-legacy hash function
    // compute_v0_13_2_hashes: we will remove this legacy check once we can verify <0.13.2 hashes.
    let is_special_trusted_case = validation.chain_id == ChainId::Mainnet && starknet_version.is_tx_hash_inconsistent();

    if is_special_trusted_case || validation.trust_transaction_hashes {
        Ok(receipts.iter().map(|r| r.transaction_hash()).collect())
    } else {
        transactions
            .par_iter()
            .enumerate()
            .map(|(index, tx)| {
                // Panic safety: receipt count was checked earlier
                let got = receipts[index].transaction_hash();
                let expected = tx.compute_hash(validation.chain_id.to_felt(), starknet_version, false);
                // compute_v0_13_2_hashes: do not check tx hash
                if !compute_v0_13_2_hashes_mode && got != expected {
                    return Err(BlockImportError::TransactionHash { index, got, expected });
                }
                Ok(got)
            })
            .collect()
    }
}

/// Compute the transaction commitment for a block.
fn transaction_commitment(
    block: &UnverifiedFullBlock,
    validation: &BlockValidationContext,
) -> Result<Felt, BlockImportError> {
    let mut starknet_version = block.header.protocol_version;

    // compute_v0_13_2_hashes mode
    let compute_v0_13_2_hashes_mode = validation.compute_v0_13_2_hashes && starknet_version < StarknetVersion::V0_13_2;
    if compute_v0_13_2_hashes_mode {
        starknet_version = StarknetVersion::V0_13_2;
    }

    let transaction_hashes = transaction_hashes(&block.receipts, &block.transactions, starknet_version, validation)?;

    if let Some(expected) = block.commitments.transaction_count {
        if expected != block.transactions.len() as u64 {
            return Err(BlockImportError::TransactionCount { got: block.transactions.len() as _, expected });
        }
    }

    // Compute transaction hashes
    let tx_hashes_with_signature: Vec<_> = block
        .transactions
        .par_iter()
        .zip(transaction_hashes)
        .map(|(tx, tx_hash)| tx.compute_hash_with_signature(tx_hash, starknet_version))
        .collect();

    // Transaction commitment
    let got = if starknet_version < StarknetVersion::V0_13_2 {
        compute_merkle_root::<Pedersen>(&tx_hashes_with_signature)
    } else {
        compute_merkle_root::<Poseidon>(&tx_hashes_with_signature)
    };

    // compute_v0_13_2_hashes: do not check old commitment
    if let Some(expected) = block.commitments.transaction_commitment {
        // compute_v0_13_2_hashes: do not check old commitment
        if !compute_v0_13_2_hashes_mode && expected != got {
            return Err(BlockImportError::TransactionCommitment { got, expected });
        }
    }

    Ok(got)
}

/// Compute the events commitment for a block.
fn event_commitment(
    block: &UnverifiedFullBlock,
    validation: &BlockValidationContext,
) -> Result<Felt, BlockImportError> {
    let mut starknet_version = block.header.protocol_version;
    // compute_v0_13_2_hashes mode
    let compute_v0_13_2_hashes_mode = validation.compute_v0_13_2_hashes && starknet_version < StarknetVersion::V0_13_2;
    if compute_v0_13_2_hashes_mode {
        starknet_version = StarknetVersion::V0_13_2;
    }

    let events_with_tx_hash: Vec<_> = block
        .receipts
        .iter()
        .flat_map(|receipt| receipt.events().iter().map(move |event| (receipt.transaction_hash(), event.clone())))
        .collect();

    if let Some(expected) = block.commitments.event_count {
        if expected != events_with_tx_hash.len() as u64 {
            return Err(BlockImportError::EventCount { got: events_with_tx_hash.len() as _, expected });
        }
    }

    let got = if events_with_tx_hash.is_empty() {
        Felt::ZERO
    } else if starknet_version < StarknetVersion::V0_13_2 {
        let events_hash =
            events_with_tx_hash.into_par_iter().map(|(_, event)| event.compute_hash_pedersen()).collect::<Vec<_>>();
        compute_merkle_root::<Pedersen>(&events_hash)
    } else {
        let events_hash = events_with_tx_hash
            .into_par_iter()
            .map(|(hash, event)| event.compute_hash_poseidon(&hash))
            .collect::<Vec<_>>();
        compute_merkle_root::<Poseidon>(&events_hash)
    };

    if let Some(expected) = block.commitments.event_commitment {
        // compute_v0_13_2_hashes: do not check old commitment
        if !compute_v0_13_2_hashes_mode && expected != got {
            return Err(BlockImportError::EventCommitment { got, expected });
        }
    }

    Ok(got)
}

/// Compute the receipt commitment for a block.
fn receipt_commitment(
    block: &UnverifiedFullBlock,
    _validation: &BlockValidationContext,
) -> Result<Felt, BlockImportError> {
    let hashes = block.receipts.par_iter().map(TransactionReceipt::compute_hash).collect::<Vec<_>>();
    let got = compute_merkle_root::<Poseidon>(&hashes);

    if let Some(expected) = block.commitments.receipt_commitment {
        if expected != got {
            return Err(BlockImportError::ReceiptCommitment { got, expected });
        }
    }
    Ok(got)
}

/// Compute the state diff commitment for a block.
fn state_diff_commitment(
    block: &UnverifiedFullBlock,
    _validation: &BlockValidationContext,
) -> Result<Felt, BlockImportError> {
    let got = block.state_diff.len() as u64;
    if let Some(expected) = block.commitments.state_diff_length {
        if expected != got {
            return Err(BlockImportError::StateDiffLength { got, expected });
        }
    }

    let got = block.state_diff.compute_hash();
    if let Some(expected) = block.commitments.state_diff_commitment {
        if expected != got {
            return Err(BlockImportError::StateDiffCommitment { got, expected });
        }
    }
    Ok(got)
}

/// Compute the root hash of a list of values.
/// This implements transactions, events, receipts and state-diff [commitments specs] using memory
/// backed bonsai storage.
///
/// [commitments specs]: https://docs.starknet.io/architecture-and-concepts/network-architecture/block-structure/#transactions_events_receipts_commitments
// The `HashMapDb` can't fail, so we can safely unwrap the results.
//
// perf: Note that committing changes still has the greatest performance hit
// as this is where the root hash is calculated. Due to the Merkle structure
// of Bonsai Tries, this results in a trie size that grows very rapidly with
// each new insertion. It seems that the only vector of optimization here
// would be to parallelize the tree traversal on insertion and optimize hash computation.
// It seems lambdaclass' crypto lib does not do simd hashing, we may want to look into that.
fn compute_merkle_root<H: StarkHash + Send + Sync>(values: &[Felt]) -> Felt {
    //TODO: replace the identifier by an empty slice when bonsai supports it
    const IDENTIFIER: &[u8] = b"0xinmemory";
    let config = bonsai_trie::BonsaiStorageConfig::default();
    let bonsai_db = bonsai_trie::databases::HashMapDb::<bonsai_trie::id::BasicId>::default();
    let mut bonsai_storage =
        bonsai_trie::BonsaiStorage::<_, _, H>::new(bonsai_db, config, /* max tree height */ 64);

    values.iter().enumerate().for_each(|(id, value)| {
        let key = BitVec::from_vec(id.to_be_bytes().to_vec());
        bonsai_storage.insert(IDENTIFIER, key.as_bitslice(), value).expect("Failed to insert into bonsai storage");
    });

    let id = bonsai_trie::id::BasicIdBuilder::new().new_id();

    bonsai_storage.commit(id).expect("Failed to commit to bonsai storage");
    bonsai_storage.root_hash(IDENTIFIER).expect("Failed to get root hash")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_root() {
        let values = vec![Felt::ONE, Felt::TWO, Felt::THREE];
        let root = compute_merkle_root::<Poseidon>(&values);

        assert_eq!(root, Felt::from_hex_unchecked("0x3b5cc7f1292eb3847c3f902d048a7e5dc7702d1c191ccd17c2d33f797e6fc32"));
    }
}
