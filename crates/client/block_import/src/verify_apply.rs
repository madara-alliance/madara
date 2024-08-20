use std::{borrow::Cow, sync::Arc};

use dc_db::{DeoxysBackend, DeoxysStorageError};
use dp_block::{
    header::PendingHeader, BlockId, BlockTag, DeoxysBlockInfo, DeoxysBlockInner, DeoxysMaybePendingBlock,
    DeoxysMaybePendingBlockInfo, DeoxysPendingBlockInfo, Header,
};
use dp_convert::ToFelt;
use starknet_api::core::ChainId;
use starknet_core::types::Felt;
use starknet_types_core::hash::{Poseidon, StarkHash};

use crate::{
    BlockImportError, BlockImportResult, PendingBlockImportResult, PreValidatedBlock, PreValidatedPendingBlock,
    RayonPool, UnverifiedHeader, ValidatedCommitments, Validation,
};

mod classes;
mod contracts;

pub struct VerifyApply {
    pool: Arc<RayonPool>,
    backend: Arc<DeoxysBackend>,
    // Only one thread at once can verify_apply.
    mutex: tokio::sync::Mutex<()>,
}

impl VerifyApply {
    pub fn new(backend: Arc<DeoxysBackend>, pool: Arc<RayonPool>) -> Self {
        Self { pool, backend, mutex: Default::default() }
    }

    /// This function wraps the [`verify_apply_inner`] step, which runs on the rayon pool, in a tokio-friendly future.
    pub async fn verify_apply(
        &self,
        block: PreValidatedBlock,
        validation: Validation,
    ) -> Result<BlockImportResult, BlockImportError> {
        let _exclusive = self.mutex.lock();

        let backend = Arc::clone(&self.backend);
        self.pool.spawn_rayon_task(move || verify_apply_inner(&backend, block, validation)).await
    }

    /// See [`Self::verify_apply`].
    pub async fn verify_apply_pending(
        &self,
        block: PreValidatedPendingBlock,
        validation: Validation,
    ) -> Result<PendingBlockImportResult, BlockImportError> {
        let _exclusive = self.mutex.lock();

        let backend = Arc::clone(&self.backend);
        self.pool.spawn_rayon_task(move || verify_apply_pending_inner(&backend, block, validation)).await
    }
}

/// This needs to be called sequentially, it will apply the state diff to the db, verify the state root and save the block.
/// This runs on the [`rayon`] threadpool however as it uses parallelism inside.
pub fn verify_apply_inner(
    backend: &DeoxysBackend,
    block: PreValidatedBlock,
    validation: Validation,
) -> Result<BlockImportResult, BlockImportError> {
    // Check block number and block hash against db
    let (block_number, parent_block_hash) =
        check_parent_hash_and_num(backend, block.header.parent_block_hash, block.unverified_block_number)?;

    // Update contract and its storage tries
    let global_state_root = update_tries(backend, &block, block_number)?;

    // Block hash
    let (block_hash, header) = block_hash(&block, &validation, block_number, parent_block_hash, global_state_root)?;

    // store block, also uses rayon heavily internally
    backend
        .store_block(
            DeoxysMaybePendingBlock {
                info: DeoxysMaybePendingBlockInfo::NotPending(DeoxysBlockInfo {
                    header: header.clone(),
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

    Ok(BlockImportResult { header, block_hash })
}

/// See [`verify_apply_inner`].
pub fn verify_apply_pending_inner(
    backend: &DeoxysBackend,
    block: PreValidatedPendingBlock,
    _validation: Validation,
) -> Result<PendingBlockImportResult, BlockImportError> {
    let (_block_number, parent_block_hash) = check_parent_hash_and_num(backend, block.header.parent_block_hash, None)?;

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
            DeoxysMaybePendingBlock {
                info: DeoxysMaybePendingBlockInfo::Pending(DeoxysPendingBlockInfo {
                    header: header.clone(),
                    tx_hashes: block.receipts.iter().map(|tx| tx.transaction_hash()).collect(),
                }),
                inner: DeoxysBlockInner { transactions: block.transactions, receipts: block.receipts },
            },
            block.state_diff,
            block.converted_classes,
        )
        .map_err(make_db_error("storing block in db"))?;

    Ok(PendingBlockImportResult {})
}

fn make_db_error(context: impl Into<Cow<'static, str>>) -> impl FnOnce(DeoxysStorageError) -> BlockImportError {
    move |error| BlockImportError::InternalDb { context: context.into(), error }
}

/// Returns the current block number and parent block hash.
fn check_parent_hash_and_num(
    backend: &DeoxysBackend,
    parent_block_hash: Option<Felt>,
    unverified_block_number: Option<u64>,
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
        if block_n != expected_block_number {
            return Err(BlockImportError::LatestBlockN { expected: expected_block_number, got: block_n });
        }
        block_n
    } else {
        expected_block_number
    };

    if let Some(parent_block_hash) = parent_block_hash {
        if parent_block_hash != expected_parent_block_hash {
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
    backend: &DeoxysBackend,
    block: &PreValidatedBlock,
    block_number: u64,
) -> Result<Felt, BlockImportError> {
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
    validation: &Validation,
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

        if expected != block_hash {
            return Err(BlockImportError::BlockHash { got: block_hash, expected });
        }
    }

    Ok((block_hash, header))
}
