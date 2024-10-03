use std::collections::HashMap;

use super::GetStorageProofResult;
use crate::{
    errors::{ProofKeyNotInTreeError, StarknetRpcApiError, StorageProofLimit, StorageProofTrie},
    utils::{display_internal_server_error, ResultExt},
    versions::v0_8_0::ProofNode,
    Starknet,
};
use bitvec::{array::BitArray, order::Msb0, slice::BitSlice};
use jsonrpsee::core::RpcResult;
use mc_db::{bonsai_identifier, BonsaiStorageError, GlobalTrie};
use starknet_core::types::{BlockId, BlockTag, Felt};
use starknet_types_core::hash::StarkHash;

fn path_to_felt(path: &BitSlice<u8, Msb0>) -> Felt {
    let mut arr = [0u8; 32];
    let slice = &mut BitSlice::from_slice_mut(&mut arr)[5..];
    slice[..path.len()].copy_from_bitslice(&path);
    Felt::from_bytes_be(&arr)
}

fn make_trie_proof<H: StarkHash + Send + Sync>(
    block_n: u64,
    trie: &mut GlobalTrie<H>,
    trie_name: StorageProofTrie,
    identifier: &[u8],
    keys: Vec<Felt>,
) -> RpcResult<HashMap<Felt, ProofNode>> {
    if !keys.is_empty() {
        let bytes = keys[0].to_bytes_be();

        let mut keys: Vec<_> = keys.into_iter().map(|f| BitArray::new(f.to_bytes_be())).collect();
        keys.sort();

        let proof =
            trie.get_multi_proof(identifier, keys.iter().map(|k| &k.as_bitslice()[5..])).map_err(|err| match err {
                BonsaiStorageError::CreateProofKeyNotInTree { key } => {
                    StarknetRpcApiError::ProofKeyNotInTree(ProofKeyNotInTreeError {
                        block_n,
                        trie: trie_name,
                        key: path_to_felt(&key),
                    })
                }
                err => {
                    display_internal_server_error(format!("Error while making storage multiproof: {err:#}"));
                    StarknetRpcApiError::InternalServerError
                }
            })?;

        Ok(proof
            .0
            .into_iter()
            .map(|(k, n)| {
                (
                    k,
                    match n {
                        mc_db::ProofNode::Binary { left, right } => ProofNode::Binary { left, right },
                        mc_db::ProofNode::Edge { child, path } => {
                            ProofNode::Edge { child, path: path_to_felt(&path), length: path.len() }
                        }
                    },
                )
            })
            .collect())
    } else {
        Ok(Default::default())
    }
}

pub fn get_storage_proof(
    starknet: &Starknet,
    block_id: Option<BlockId>,
    class_hashes: Vec<Felt>,
    contract_addresses: Vec<Felt>,
    contracts_storage_keys: HashMap<Felt, Vec<Felt>>,
) -> RpcResult<GetStorageProofResult> {
    // Pending block does not have a state root, so always fallbacck to latest.
    let block_id = match block_id {
        Some(BlockId::Tag(BlockTag::Pending)) | None => BlockId::Tag(BlockTag::Latest),
        Some(block_id) => block_id,
    };

    let block_n = starknet
        .backend
        .get_block_n(&block_id)
        .or_internal_server_error("Getting latest block in db")?
        .ok_or(StarknetRpcApiError::NoBlocks)?;

    // todo: configurable
    const MAX_PROOF_KEYS: usize = 1024;
    const MAX_TRIES: usize = 5;

    let proof_keys = class_hashes.len()
        + contract_addresses.len()
        + contracts_storage_keys.iter().map(|(_, v)| v.len()).sum::<usize>();
    if proof_keys > MAX_PROOF_KEYS {
        return Err(StarknetRpcApiError::ProofLimitExceeded {
            kind: StorageProofLimit::MaxKeys,
            limit: MAX_PROOF_KEYS,
            got: proof_keys,
        }
        .into());
    }

    let n_tries = (!class_hashes.is_empty() as usize)
        + (!contract_addresses.is_empty() as usize)
        + contracts_storage_keys.iter().map(|(_, keys)| (!keys.is_empty() as usize)).sum::<usize>();
    if n_tries > MAX_TRIES {
        return Err(StarknetRpcApiError::ProofLimitExceeded {
            kind: StorageProofLimit::MaxUsedTries,
            limit: MAX_TRIES,
            got: n_tries,
        }
        .into());
    }

    let classes_proof = make_trie_proof(
        block_n,
        &mut starknet.backend.class_trie(),
        StorageProofTrie::Classes,
        bonsai_identifier::CLASS,
        class_hashes,
    )?;
    let contracts_proof = make_trie_proof(
        block_n,
        &mut starknet.backend.contract_trie(),
        StorageProofTrie::Contracts,
        bonsai_identifier::CONTRACT,
        contract_addresses,
    )?;

    let contracts_storage_proofs = contracts_storage_keys
        .into_iter()
        .map(|(contract_address, keys)| {
            let identifier = contract_address.to_bytes_be();
            Ok((
                contract_address,
                make_trie_proof(
                    block_n,
                    &mut starknet.backend.contract_storage_trie(),
                    StorageProofTrie::ContractStorage(contract_address),
                    &identifier,
                    keys,
                )?,
            ))
        })
        .collect::<RpcResult<_>>()?;

    Ok(GetStorageProofResult { classes_proof, contracts_proof, contracts_storage_proofs })
}
