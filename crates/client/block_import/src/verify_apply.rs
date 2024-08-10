use std::borrow::Cow;

use dc_db::{DeoxysBackend, DeoxysStorageError};
use dp_block::{
    BlockId, BlockTag, DeoxysBlockInfo, DeoxysBlockInner, DeoxysMaybePendingBlock, DeoxysMaybePendingBlockInfo, Header,
};
use dp_convert::ToFelt;
use starknet_core::types::Felt;
use starknet_types_core::hash::{Poseidon, StarkHash};

use crate::{
    pre_validate::{BlockCommitments, PreValidatedBlock},
    BlockImportError, UnverifiedHeader, Validation,
};

mod classes;
mod contracts;

/// "STARKNET_STATE_V0"
const STARKNET_STATE_PREFIX: Felt = Felt::from_hex_unchecked("0x535441524b4e45545f53544154455f5630");

fn calculate_state_root(contracts_trie_root: Felt, classes_trie_root: Felt) -> Felt {
    if classes_trie_root == Felt::ZERO {
        contracts_trie_root
    } else {
        Poseidon::hash_array(&[STARKNET_STATE_PREFIX, contracts_trie_root, classes_trie_root])
    }
}

fn make_db_error(context: impl Into<Cow<'static, str>>) -> impl FnOnce(DeoxysStorageError) -> BlockImportError {
    move |error| BlockImportError::InternalDb { context: context.into(), error }
}

// TODO: enforce sequential order with a global lock
/// This needs to be called sequentially, it will apply the state diff to the db, verify the state root and save the block.
/// This runs on the [`rayon`] threadpool however as it uses parallelism inside.
pub fn verify_apply(
    backend: &DeoxysBackend,
    block: PreValidatedBlock,
    validation: &Validation,
) -> Result<(), BlockImportError> {
    // Check block number and block hash against db

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

    if block.header.block_number != expected_block_number {
        return Err(BlockImportError::LatestBlockN { expected: expected_block_number, got: block.header.block_number });
    }
    if block.header.parent_block_hash != expected_parent_block_hash {
        return Err(BlockImportError::ParentHash {
            expected: expected_parent_block_hash,
            got: block.header.parent_block_hash,
        });
    }

    // Update contract and its storage tries
    let (contract_trie_root, class_trie_root) = rayon::join(
        || {
            contracts::contract_trie_root(
                backend,
                &block.state_diff.deployed_contracts,
                &block.state_diff.replaced_classes,
                &block.state_diff.nonces,
                &block.state_diff.storage_diffs,
                block.header.block_number,
            )
        },
        || classes::class_trie_root(backend, &block.state_diff.declared_classes, block.header.block_number),
    );

    let state_root = calculate_state_root(
        contract_trie_root.map_err(make_db_error("updating contract trie root"))?,
        class_trie_root.map_err(make_db_error("updating class trie root"))?,
    );
    if let Some(expected) = validation.global_state_root {
        if expected != state_root {
            return Err(BlockImportError::GlobalStateRoot { got: state_root, expected });
        }
    }

    // Now, block hash

    let UnverifiedHeader {
        parent_block_hash,
        block_number,
        sequencer_address,
        block_timestamp,
        protocol_version,
        l1_gas_price,
        l1_da_mode,
    } = block.header;

    let BlockCommitments {
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
        global_state_root: state_root,
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

    if let Some(expected) = validation.block_hash {
        if expected != state_root {
            return Err(BlockImportError::GlobalStateRoot { got: state_root, expected });
        }
    }

    // store block, also uses rayon heavily internally
    backend
        .store_block(
            DeoxysMaybePendingBlock {
                info: DeoxysMaybePendingBlockInfo::NotPending(DeoxysBlockInfo {
                    header,
                    block_hash,
                    // get tx hashes from receipts, they have been validated in pre_validate.
                    tx_hashes: block.receipts.iter().map(|tx| tx.transaction_hash()).collect(),
                }),
                inner: DeoxysBlockInner { transactions: block.transactions, receipts: block.receipts },
            },
            block.state_diff,
            block.converted_classes,
        )
        .map_err(make_db_error("storing block in db"))?;

    Ok(())
}
