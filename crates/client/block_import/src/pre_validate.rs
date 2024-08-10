use crate::{BlockImportError, DeclaredClass, UnverifiedFullBlock, UnverifiedHeader, Validation};
use bitvec::vec::BitVec;
use dp_chain_config::StarknetVersion;
use dp_class::{ClassInfo, ConvertedClass, ToCompiledClass};
use dp_convert::ToFelt;
use dp_receipt::TransactionReceipt;
use dp_state_update::StateDiff;
use dp_transactions::Transaction;
use dp_utils::spawn_rayon_task;
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator};
use rayon::prelude::*;
use starknet_core::types::Felt;
use starknet_types_core::hash::{Pedersen, Poseidon, StarkHash};
use std::mem;

/// Pre-validate a block, computing and checking the commitments
pub async fn pre_validate(
    block: UnverifiedFullBlock,
    validation: &Validation,
) -> Result<PreValidatedBlock, BlockImportError> {
    let validation = validation.clone();
    spawn_rayon_task(move || pre_validate_inner(block, &validation)).await
}

#[derive(Default)]
pub struct BlockCommitments {
    pub transaction_count: u64,
    pub transaction_commitment: Felt,
    pub event_count: u64,
    pub event_commitment: Felt,
    pub state_diff_length: u64,
    pub state_diff_commitment: Felt,
    pub receipt_commitment: Felt,
}

pub struct PreValidatedBlock {
    pub header: UnverifiedHeader,
    pub transactions: Vec<Transaction>,
    pub state_diff: StateDiff,
    pub receipts: Vec<TransactionReceipt>,
    pub commitments: BlockCommitments,
    pub converted_classes: Vec<ConvertedClass>,
}

/// This runs on the [`rayon`] threadpool.
fn pre_validate_inner(
    mut block: UnverifiedFullBlock,
    validation: &Validation,
) -> Result<PreValidatedBlock, BlockImportError> {
    let classes = mem::take(&mut block.declared_classes);

    // unfortunately this is ugly but rayon::join does not have the fast error short circuiting behavior that
    // collecting into a Result has.
    // little known fact this uses the impl FromIterator for () from std, nice trick
    let (mut commitments, mut converted_classes) = Default::default();
    [
        Box::new(|| Ok(commitments = block_commitments(&block, validation)?))
            as Box<dyn FnOnce() -> Result<(), BlockImportError> + Send>,
        Box::new(|| Ok(converted_classes = convert_classes(validation, classes)?)),
    ]
    .into_par_iter()
    .map(|f| f())
    .collect::<Result<(), _>>()?;

    Ok(PreValidatedBlock {
        header: block.header,
        transactions: block.transactions,
        state_diff: block.state_diff,
        receipts: block.receipts,
        commitments,
        converted_classes,
    })
}

fn block_commitments(
    block: &UnverifiedFullBlock,
    validation: &Validation,
) -> Result<BlockCommitments, BlockImportError> {
    let (mut receipt_c, mut state_diff_c, mut transaction_c, mut event_c) = Default::default();
    [
        Box::new(|| Ok(receipt_c = receipt_commitment(&block, validation)?))
            as Box<dyn FnOnce() -> Result<(), BlockImportError> + Send>,
        Box::new(|| Ok(state_diff_c = state_diff_commitment(&block, validation)?)),
        Box::new(|| Ok(transaction_c = transaction_commitment(&block, validation)?)),
        Box::new(|| Ok(event_c = event_commitment(&block, validation)?)),
    ]
    .into_par_iter()
    .map(|f| f())
    .collect::<Result<(), _>>()?;

    Ok(BlockCommitments {
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
    validation: &Validation,
    declared_classes: Vec<DeclaredClass>,
) -> Result<Vec<ConvertedClass>, BlockImportError> {
    declared_classes.into_par_iter().map(|class| class_conversion(validation, class)).collect()
}

fn class_conversion(_validation: &Validation, class: DeclaredClass) -> Result<ConvertedClass, BlockImportError> {
    let DeclaredClass { class_hash, contract_class, compiled_class_hash } = class;

    // TODO(class_hash): uncomment this when the class hashes are computed correctly accross the entire state
    // let expected =
    //     contract_class.class_hash().map_err(|e| BlockImportError::ComputeClassHashError(e.to_string()))?;
    // if class_hash != expected {
    // }

    let compiled_class = contract_class
        .compile()
        .map_err(|e| BlockImportError::CompilationClassError { error: e.to_string(), class_hash })?;

    let class_info = ClassInfo { contract_class: contract_class.into(), compiled_class_hash };

    Ok(ConvertedClass { class_infos: (class_hash, class_info), class_compiled: (class_hash, compiled_class) })
}

/// Compute the transaction commitment for a block.
fn transaction_commitment(block: &UnverifiedFullBlock, validation: &Validation) -> Result<Felt, BlockImportError> {
    let block_number = block.header.block_number;
    let starknet_version = block.header.protocol_version;

    if block.receipts.len() != block.transactions.len() {
        return Err(BlockImportError::TransactionEqualReceiptCount {
            receipts: block.receipts.len(),
            transactions: block.transactions.len(),
        });
    }

    // Compute transaction hashes
    let tx_hashes_with_signature: Vec<_> = block
        .transactions
        .par_iter()
        .enumerate()
        .map(|(index, tx)| {
            // Panic safety: receipt count was checked earlier
            let tx_hash = block.receipts[index].transaction_hash();
            if !validation.trust_transaction_hashes {
                let expected = tx.compute_hash(validation.chain_id.to_felt(), false, Some(block_number));
                if tx_hash != expected {
                    return Err(BlockImportError::TransactionHash { index, got: tx_hash, expected });
                }
            }
            let tx_hash_with_signature = tx.compute_hash_with_signature(tx_hash, starknet_version);
            Ok(tx_hash_with_signature)
        })
        .collect::<Result<_, BlockImportError>>()?;

    // Transaction commitment
    let root = if starknet_version < StarknetVersion::STARKNET_VERSION_0_13_2 {
        compute_merkle_root::<Pedersen>(&tx_hashes_with_signature)
    } else {
        compute_merkle_root::<Poseidon>(&tx_hashes_with_signature)
    };

    Ok(root)
}

/// Compute the events commitment for a block.
fn event_commitment(block: &UnverifiedFullBlock, _validation: &Validation) -> Result<Felt, BlockImportError> {
    let events_with_tx_hash: Vec<_> = block
        .receipts
        .iter()
        .flat_map(|receipt| receipt.events().iter().map(move |event| (receipt.transaction_hash(), event.clone())))
        .collect();

    if events_with_tx_hash.is_empty() {
        return Ok(Felt::ZERO);
    }

    if block.header.protocol_version < StarknetVersion::STARKNET_VERSION_0_13_2 {
        let events_hash =
            events_with_tx_hash.into_par_iter().map(|(_, event)| event.compute_hash_pedersen()).collect::<Vec<_>>();
        Ok(compute_merkle_root::<Pedersen>(&events_hash))
    } else {
        let events_hash = events_with_tx_hash
            .into_par_iter()
            .map(|(hash, event)| event.compute_hash_poseidon(&hash))
            .collect::<Vec<_>>();
        Ok(compute_merkle_root::<Poseidon>(&events_hash))
    }
}

/// Compute the receipt commitment for a block.
fn receipt_commitment(block: &UnverifiedFullBlock, validation: &Validation) -> Result<Felt, BlockImportError> {
    let hashes = block.receipts.par_iter().map(TransactionReceipt::compute_hash).collect::<Vec<_>>();
    let got = compute_merkle_root::<Poseidon>(&hashes);

    if let Some(expected) = validation.receipt_commitment {
        if expected != got {
            return Err(BlockImportError::ReceiptCommitment { got, expected });
        }
    }
    Ok(got)
}

/// Compute the state diff commitment for a block.
fn state_diff_commitment(block: &UnverifiedFullBlock, validation: &Validation) -> Result<Felt, BlockImportError> {
    let got = block.state_diff.len() as u64;
    if let Some(expected) = validation.state_diff_length {
        if expected != got {
            return Err(BlockImportError::StateDiffLength { got, expected });
        }
    }

    let got = block.state_diff.compute_hash();
    if let Some(expected) = validation.state_diff_commitment {
        if expected != got {
            return Err(BlockImportError::StateDiffCommitment { got, expected });
        }
    }
    Ok(got)
}

/// Compute the root hash of a list of values.
// The `HashMapDb` can't fail, so we can safely unwrap the results.
fn compute_merkle_root<H: StarkHash + Send + Sync>(values: &[Felt]) -> Felt {
    //TODO: replace the identifier by an empty slice when bonsai supports it
    const IDENTIFIER: &[u8] = b"0xinmemory";
    let config = bonsai_trie::BonsaiStorageConfig::default();
    let bonsai_db = bonsai_trie::databases::HashMapDb::<bonsai_trie::id::BasicId>::default();
    let mut bonsai_storage =
        bonsai_trie::BonsaiStorage::<_, _, H>::new(bonsai_db, config).expect("Failed to create bonsai storage");

    values.iter().enumerate().for_each(|(id, value)| {
        let key = BitVec::from_vec(id.to_be_bytes().to_vec());
        bonsai_storage.insert(IDENTIFIER, key.as_bitslice(), value).expect("Failed to insert into bonsai storage");
    });

    // Note that committing changes still has the greatest performance hit
    // as this is where the root hash is calculated. Due to the Merkle structure
    // of Bonsai Tries, this results in a trie size that grows very rapidly with
    // each new insertion. It seems that the only vector of optimization here
    // would be to optimize the tree traversal and hash computation.
    let id = bonsai_trie::id::BasicIdBuilder::new().new_id();

    // run in a blocking-safe thread to avoid starving the thread pool
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
