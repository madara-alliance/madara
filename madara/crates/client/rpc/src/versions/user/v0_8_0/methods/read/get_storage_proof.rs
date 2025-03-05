use crate::{
    errors::{StarknetRpcApiError, StorageProofLimit, StorageProofTrie},
    utils::ResultExt,
    versions::user::v0_8_0::{
        ContractLeavesDataItem, ContractStorageKeysItem, ContractsProof, GetStorageProofResult, GlobalRoots,
        MerkleNode, NodeHashToNodeMappingItem,
    },
    Starknet,
};
use bitvec::{array::BitArray, order::Msb0, slice::BitSlice};
use jsonrpsee::core::RpcResult;
use mc_db::{bonsai_identifier, db_block_id::DbBlockId, BasicId, GlobalTrie};
use mp_block::{BlockId, BlockTag};
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::StarkHash;
use std::iter;

fn saturating_sum(iter: impl IntoIterator<Item = usize>) -> usize {
    iter.into_iter().fold(0, |acc, cur| acc.saturating_add(cur))
}

fn path_to_felt(path: &BitSlice<u8, Msb0>) -> Felt {
    let mut arr = [0u8; 32];
    let slice = &mut BitSlice::from_slice_mut(&mut arr)[5..];
    let slice_len = slice.len();
    slice[slice_len - path.len()..].copy_from_bitslice(path);
    Felt::from_bytes_be(&arr)
}

/// Returns (root hash, nodes)
fn make_trie_proof<H: StarkHash + Send + Sync>(
    block_n: u64,
    trie: &mut GlobalTrie<H>,
    trie_name: StorageProofTrie,
    identifier: &[u8],
    keys: Vec<Felt>,
) -> RpcResult<(Felt, Vec<NodeHashToNodeMappingItem>)> {
    let mut keys: Vec<_> = keys.into_iter().map(|f| BitArray::new(f.to_bytes_be())).collect();
    keys.sort();

    tracing::debug!("Getting trie proof for {trie_name:?} on block {block_n} for n={} keys", keys.len());

    let mut storage = trie
        .get_transactional_state(BasicId::new(block_n), trie.get_config())
        .map_err(|err| anyhow::anyhow!("{err:#}"))
        .or_internal_server_error("Getting transactional state")?
        .ok_or(StarknetRpcApiError::CannotMakeProofOnOldBlock)?;

    let root_hash = storage
        .root_hash(identifier)
        .map_err(|err| anyhow::anyhow!("{err:#}"))
        .or_internal_server_error("Getting root hash of trie")?;

    let proof = storage
        .get_multi_proof(identifier, keys.iter().map(|k| &k.as_bitslice()[5..]))
        .map_err(|err| anyhow::anyhow!("{err:#}"))
        .or_internal_server_error("Error while making storage multiproof")?;

    // convert the bonsai-trie type to the rpc DTO
    let converted_proof = proof
        .0
        .into_iter()
        .map(|(node_hash, n)| {
            let node = match n {
                mc_db::ProofNode::Binary { left, right } => MerkleNode::Binary { left, right },
                mc_db::ProofNode::Edge { child, path } => {
                    MerkleNode::Edge { child, path: path_to_felt(&path), length: path.len() }
                }
            };
            NodeHashToNodeMappingItem { node_hash, node }
        })
        .collect();

    Ok((root_hash, converted_proof))
}

pub fn get_storage_proof(
    starknet: &Starknet,
    block_id: BlockId,
    class_hashes: Option<Vec<Felt>>,
    contract_addresses: Option<Vec<Felt>>,
    contracts_storage_keys: Option<Vec<ContractStorageKeysItem>>,
) -> RpcResult<GetStorageProofResult> {
    // Pending block does not have a state root, so always fallback to latest.
    let block_id = match block_id {
        BlockId::Tag(BlockTag::Pending) => BlockId::Tag(BlockTag::Latest),
        block_id => block_id,
    };

    let block_n = starknet
        .backend
        .get_block_n(&block_id)
        .or_internal_server_error("Resolving block number")?
        .ok_or(StarknetRpcApiError::NoBlocks)?;

    let Some(latest) = starknet.backend.get_latest_block_n().or_internal_server_error("Getting latest block in db")?
    else {
        return Err(StarknetRpcApiError::BlockNotFound.into());
    };

    if latest.saturating_sub(block_n) > starknet.storage_proof_config.max_distance {
        return Err(StarknetRpcApiError::CannotMakeProofOnOldBlock.into());
    }

    let block_hash = starknet
        .backend
        .get_block_hash(&block_id)
        .or_internal_server_error("Resolving block hash")?
        .ok_or(StarknetRpcApiError::NoBlocks)?;

    let class_hashes = class_hashes.unwrap_or_default();
    let contract_addresses = contract_addresses.unwrap_or_default();
    let contracts_storage_keys = contracts_storage_keys.unwrap_or_default();

    // Check limits.

    let proof_keys = saturating_sum(
        iter::once(class_hashes.len())
            .chain(iter::once(contract_addresses.len()))
            .chain(contracts_storage_keys.iter().map(|v| v.storage_keys.len())),
    );
    if proof_keys > starknet.storage_proof_config.max_keys {
        return Err(StarknetRpcApiError::ProofLimitExceeded {
            kind: StorageProofLimit::MaxKeys,
            limit: starknet.storage_proof_config.max_keys,
            got: proof_keys,
        }
        .into());
    }

    let n_tries = saturating_sum(
        iter::once(!class_hashes.is_empty() as usize)
            .chain(iter::once(!contract_addresses.is_empty() as usize))
            .chain(contracts_storage_keys.iter().map(|keys| (!keys.storage_keys.is_empty() as usize))),
    );
    if n_tries > starknet.storage_proof_config.max_tries {
        return Err(StarknetRpcApiError::ProofLimitExceeded {
            kind: StorageProofLimit::MaxUsedTries,
            limit: starknet.storage_proof_config.max_tries,
            got: n_tries,
        }
        .into());
    }

    // Make the proofs.

    let (classes_tree_root, classes_proof) = make_trie_proof(
        block_n,
        &mut starknet.backend.class_trie(),
        StorageProofTrie::Classes,
        bonsai_identifier::CLASS,
        class_hashes,
    )?;

    let mut contract_root_hashes = std::collections::HashMap::new();
    let contracts_storage_proofs = contracts_storage_keys
        .into_iter()
        .map(|ContractStorageKeysItem { contract_address, storage_keys }| {
            let identifier = contract_address.to_bytes_be();
            let (root_hash, proof) = make_trie_proof(
                block_n,
                &mut starknet.backend.contract_storage_trie(),
                StorageProofTrie::ContractStorage(contract_address),
                &identifier,
                storage_keys,
            )?;
            contract_root_hashes.insert(contract_address, root_hash);
            Ok(proof)
        })
        .collect::<RpcResult<_>>()?;

    // contract leaves data
    let contract_leaves_data = contract_addresses
        .iter()
        .map(|contract_addr| {
            Ok(ContractLeavesDataItem {
                nonce: starknet
                    .backend
                    .get_contract_nonce_at(&DbBlockId::Number(block_n), contract_addr)
                    .or_internal_server_error("Getting contract nonce")?
                    .unwrap_or(Felt::ZERO),
                class_hash: starknet
                    .backend
                    .get_contract_class_hash_at(&DbBlockId::Number(block_n), contract_addr)
                    .or_internal_server_error("Getting contract class hash")?
                    .unwrap_or(Felt::ZERO),
                storage_root: *contract_root_hashes
                    .get(contract_addr)
                    .unwrap_or(&Felt::ZERO),
            })
        })
        .collect::<RpcResult<_>>()?;
    let (contracts_tree_root, contracts_proof_nodes) = make_trie_proof(
        block_n,
        &mut starknet.backend.contract_trie(),
        StorageProofTrie::Contracts,
        bonsai_identifier::CONTRACT,
        contract_addresses,
    )?;

    let contracts_proof = ContractsProof { nodes: contracts_proof_nodes, contract_leaves_data };

    Ok(GetStorageProofResult {
        classes_proof,
        contracts_proof,
        contracts_storage_proofs,
        global_roots: GlobalRoots { contracts_tree_root, classes_tree_root, block_hash },
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use bitvec::{bits, vec::BitVec, view::{AsBits, BitView as _}};
    use blockifier::execution::contract_address;
    use mc_db::tests::common::finalized_block_one;
    use mp_state_update::{ContractStorageDiffItem, StateDiff, StorageEntry};
    use starknet_types_core::hash::{Pedersen, Poseidon};

    use super::*;

    use crate::test_utils::rpc_test_setup;
    use mc_block_import::tests::block_import_utils::create_dummy_header;

    #[test]
    fn test_path_to_felt() {
        let path = bits![u8, Msb0; 0, 0];
        assert_eq!(path.len(), 2);
        let felt = path_to_felt(path);
        assert_eq!(felt, Felt::ZERO);

        let path = bits![u8, Msb0; 1];
        assert_eq!(path.len(), 1);
        let felt = path_to_felt(path);
        assert_eq!(felt, Felt::ONE);
    }

    #[tokio::test]
    #[rstest::rstest]
    #[case(vec![(Felt::TWO, Felt::ONE, Felt::THREE)])]
    async fn test_contract_storage_trie_proof(
        #[case] storage_items: Vec<(Felt, Felt, Felt)>,
        rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, Starknet)
    ) {
        use std::collections::HashMap;

        let (_backend, starknet) = rpc_test_setup;

        let mut storage_trie = starknet.backend.contract_storage_trie();
        let mut contract_storage: HashMap<Felt, Vec<(Felt, Felt)>> = HashMap::new();
        let mut contract_addresses = Vec::new();
        let mut contract_storage_keys: HashMap<Felt, Vec<Felt>> = HashMap::new();

        // for each triplet (contract_address, storage_key, value) we insert the k:v pair into the
        // bonsai trie for that contract and also prepare some other data we will use later
        for (contract_address, storage_key, value) in &storage_items {
            storage_trie.insert(
                &contract_address.to_bytes_be(),
                &storage_key.to_bytes_be().as_bits()[5..].to_owned(),
                &value,
            ).unwrap();

            // also use this to map out the k:v storage pairs for each contract we're proving
            contract_storage.entry(*contract_address).or_default().push((*storage_key, *value));

            // prepare input for get_storage_proof
            contract_addresses.push(*contract_address);
            contract_storage_keys.entry(*contract_address).or_default().push(*storage_key);

        }
        storage_trie.commit(BasicId::new(1));

        // create a dummy block to make get_storage_proof() happy
        // (it wants a block to exist for the requested chain height)
        let header = create_dummy_header();
        let pending_block = finalized_block_one();
        starknet.backend.store_block(
            pending_block,
            StateDiff::default(),
            vec![],
            None,
            None,
        ).unwrap();

        // convert contract_storage_keys to vec of ContractStorageKeyItems now that we have all keys
        let contract_storage_keys = contract_storage_keys.into_iter().map(|(contract_address, storage_keys)| {
            ContractStorageKeysItem { contract_address, storage_keys }
        })
        .collect();

        let storage_proof_result = get_storage_proof(
            &starknet,
            BlockId::Tag(BlockTag::Latest),
            None,
            Some(contract_addresses),
            Some(contract_storage_keys),
        ).unwrap();

        // the contract storage roots are buried in the unordered Vec<ContracTLeavesDataItem>, we need each
        // root so we convert to a hash map
        let mut index = 0;
        let storage_roots = storage_proof_result.contracts_proof.contract_leaves_data.into_iter().map(|contract_leaves_data_item| {
            // TODO: we don't get contract_address anywhere in the proof (except, techincally, for
            //       the path itself to a leaf), so we assume the vec order is the same as what we
            //       requested.
            // TODO: but even this is wrong in a case where we ask for multiple storage items in a
            //       single contract
            let contract_address = storage_items[index].0;
            (contract_address, contract_leaves_data_item.storage_root)
        })
        .collect::<HashMap<_, _>>();

        // for each contract we have a proof for, walk through the proof for all storage keys requested
        for contract_address in contract_storage.keys() {
            let storage_root = storage_roots
                .get(contract_address)
                .expect(format!("no proof returned for contract {:x}", contract_address).as_str());

            // TODO: go through all keys
            // TODO: get proof for this contract
            let key = storage_items[0].1;
            verify_proofs(storage_root, &key, &storage_proof_result.contracts_storage_proofs[0]).expect("verify_proofs failed");
        }
    }

    // copied from bonsai-trie and modified to avoid unneeded types
    pub fn hash_binary_node<H: StarkHash>(left_hash: Felt, right_hash: Felt) -> Felt {
        H::hash(&left_hash, &right_hash)
    }
    pub fn hash_edge_node<H: StarkHash>(path: &Felt, path_length: usize, child_hash: Felt) -> Felt {
        let path_bitslice: &BitSlice<_, Msb0> = &BitVec::from_slice(&path.to_bytes_be());
        assert!(path_bitslice.len() == 256, "Felt::to_bytes_be() expected to always be 256 bits");

        let felt_path = path;
        let mut length = [0; 32];
        // Safe as len() is guaranteed to be <= 251
        length[31] = path_length as u8;

        let length = Felt::from_bytes_be(&length);
        H::hash(&child_hash, &felt_path) + length
    }

    // TODO: document
    pub fn verify_proofs(commitment: &Felt, path: &Felt, proofs: &Vec<NodeHashToNodeMappingItem>) -> Result<Vec<MerkleNode>, String> {
        // convert vec into a hash map so we can look nodes up efficiently
        // TODO: do this outside this fn
        let proof_nodes: HashMap<Felt, MerkleNode> = proofs.iter().map(|mapping_item| {
            (mapping_item.node_hash, mapping_item.node.clone())
        })
        .collect();

        let start = 5; // 256 minus 251
        let mut index = start;
        let path_bits: BitVec<_, Msb0> = BitVec::from_slice(&path.to_bytes_be());

        let mut next_node_hash = commitment;
        let mut ordered_proof = Vec::new();
        loop {
            let node = proof_nodes
                .get(&next_node_hash)
                .ok_or(format!("proof did not contain preimage for node {:x}", next_node_hash))?;
            match node {
                MerkleNode::Binary { left, right } => {
                    next_node_hash = if path_bits[index] { right } else { left };
                    // TODO: verify node hash
                    index += 1;
                },
                MerkleNode::Edge { child, path, length } => {
                    // TODO: verify that the edge path matches the relevant part of our path
                    let actual_node_hash = hash_edge_node::<Pedersen>(path, *length, *child);
                    if &actual_node_hash != next_node_hash {
                        return Err(format!("incorrect node hash (expected {:x}, but got {:x})", next_node_hash, actual_node_hash));
                    }
                    next_node_hash = child;
                    index += length;
                },
            }

            ordered_proof.push(node.clone());

            if index > 256 {
                return Err(format!("invalid proof, path too long ({})", (index - start)));
            }
            if index == 256 {
                // TODO: verify final node
                break;
            }
        }

        Ok(ordered_proof)
    }
}
