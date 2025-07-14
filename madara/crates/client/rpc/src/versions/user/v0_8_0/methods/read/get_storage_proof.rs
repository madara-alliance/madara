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

    let Some(latest) = starknet.backend.get_block_n_latest() else {
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
                storage_root: *contract_root_hashes.get(contract_addr).unwrap_or(&Felt::ZERO),
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

    use bitvec::{bits, vec::BitVec, view::AsBits};
    use mc_db::tests::common::block;
    use mp_state_update::StateDiff;
    use starknet_types_core::hash::Pedersen;

    use super::*;

    use crate::test_utils::rpc_test_setup;

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

    struct ContractStorageTestInput {
        pub contract_address: Felt,
        pub storage_key: Felt,
        pub value: Felt,
    }

    impl ContractStorageTestInput {
        fn new(a: Felt, k: Felt, v: Felt) -> Self {
            Self { contract_address: a, storage_key: k, value: v }
        }
    }

    #[rstest::rstest]
    #[case::single_mpt_single_value(vec![
        ContractStorageTestInput::new(Felt::TWO, Felt::ONE, Felt::THREE)
    ])]
    #[case::single_mpt_many_values(vec![
        ContractStorageTestInput::new(Felt::TWO, Felt::ONE, Felt::THREE),
        ContractStorageTestInput::new(Felt::TWO, Felt::TWO, Felt::THREE),
        ContractStorageTestInput::new(Felt::TWO, Felt::from(5), Felt::from(55)),
        ContractStorageTestInput::new(Felt::TWO, Felt::from(6), Felt::from(66)),
        ContractStorageTestInput::new(Felt::TWO, Felt::from(7), Felt::from(77)),
        ContractStorageTestInput::new(Felt::TWO, Felt::from(8), Felt::from(88)),
    ])]
    #[case::multi_mpt_single_values(vec![
        ContractStorageTestInput::new(Felt::TWO, Felt::ONE, Felt::THREE),
        ContractStorageTestInput::new(Felt::from(222), Felt::from(5), Felt::from(55)),
    ])]
    #[case::multi_mpt_many_values(vec![
        ContractStorageTestInput::new(Felt::from(222), Felt::from(5), Felt::from(55)),
        ContractStorageTestInput::new(Felt::from(222), Felt::from(6), Felt::from(66)),
        ContractStorageTestInput::new(Felt::from(222), Felt::from(7), Felt::from(77)),
        ContractStorageTestInput::new(Felt::from(333), Felt::from(5), Felt::from(55)),
        ContractStorageTestInput::new(Felt::from(333), Felt::from(6), Felt::from(66)),
        ContractStorageTestInput::new(Felt::from(333), Felt::from(7), Felt::from(77)),
    ])]
    #[case::dual_mpt_duplicate_values(vec![
        ContractStorageTestInput::new(Felt::from(222), Felt::from(5), Felt::from(55)),
        ContractStorageTestInput::new(Felt::from(333), Felt::from(5), Felt::from(55)),
    ])]
    #[case::multi_mpt_duplicate_values(vec![
        ContractStorageTestInput::new(Felt::from(222), Felt::from(5), Felt::from(55)),
        ContractStorageTestInput::new(Felt::from(333), Felt::from(5), Felt::from(55)),
        ContractStorageTestInput::new(Felt::from(444), Felt::from(5), Felt::from(55)),
        ContractStorageTestInput::new(Felt::from(555), Felt::from(5), Felt::from(55)),
    ])]
    #[tokio::test]
    /// Tests `get_storage_proof()` as it relates to contract storage tries. This includes testing
    /// multiple contract storage MPTs to ensure that it provides proofs for each.
    async fn test_contract_storage_trie_proof(
        #[case] storage_items: Vec<ContractStorageTestInput>,
        #[with(1)] block: mp_block::MadaraMaybePendingBlock,
        rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, Starknet),
    ) -> Result<(), String> {
        let (_backend, starknet) = rpc_test_setup;

        let mut storage_trie = starknet.backend.contract_storage_trie();
        let mut contract_storage: HashMap<Felt, Vec<(Felt, Felt)>> = HashMap::new();
        let mut contract_addresses = Vec::new();
        let mut contract_storage_keys: HashMap<Felt, Vec<Felt>> = HashMap::new();

        // for each triplet (contract_address, storage_key, value) we insert the k:v pair into the
        // bonsai trie for that contract and also prepare some other data we will use later
        for storage_item in storage_items {
            storage_trie
                .insert(
                    &storage_item.contract_address.to_bytes_be(),
                    &storage_item.storage_key.to_bytes_be().as_bits()[5..],
                    &storage_item.value,
                )
                .unwrap();

            // also use this to map out the k:v storage pairs for each contract we're proving
            contract_storage
                .entry(storage_item.contract_address)
                .or_default()
                .push((storage_item.storage_key, storage_item.value));

            // prepare input for get_storage_proof
            if !contract_addresses.contains(&storage_item.contract_address) {
                contract_addresses.push(storage_item.contract_address);
            }
            contract_storage_keys.entry(storage_item.contract_address).or_default().push(storage_item.storage_key);
        }
        storage_trie.commit(BasicId::new(1)).expect("failed to commit to storage_trie");

        // create a dummy block to make get_storage_proof() happy
        // (it wants a block to exist for the requested chain height)
        starknet.backend.store_block(block, StateDiff::default(), vec![]).unwrap();

        // convert contract_storage_keys to vec of ContractStorageKeyItems now that we have all keys
        let contract_storage_keys_items = contract_storage_keys
            .clone()
            .into_iter()
            .map(|(contract_address, storage_keys)| ContractStorageKeysItem { contract_address, storage_keys })
            .collect();

        let storage_proof_result = get_storage_proof(
            &starknet,
            BlockId::Tag(BlockTag::Latest),
            None,
            Some(contract_addresses.clone()),
            Some(contract_storage_keys_items),
        )
        .unwrap();

        // the contract storage roots are buried in the unordered Vec<ContracTLeavesDataItem>, we need each
        // root so we convert to a hash map
        let mut index = 0;
        let storage_roots = storage_proof_result
            .contracts_proof
            .contract_leaves_data
            .into_iter()
            .map(|contract_leaves_data_item| {
                // TODO: we don't get contract_address anywhere in the proof (except, techincally, for
                //       the path itself to a leaf), so we assume the vec order is the same as what we
                //       requested.
                let contract_address = &contract_addresses[index];
                index += 1;
                (contract_address, contract_leaves_data_item.storage_root)
            })
            .collect::<HashMap<_, _>>();

        // collect all proof nodes into one big hash map. since the keys are hashes of the nodes
        // themselves, there should be no collisions. Duplicates are normal since we are asking for
        // multiple proofs out of the same MPT, but they should be identical k:v pairs (as opposed
        // to a collision where k is identical but v is not).
        let mut proof_nodes = HashMap::new();
        for node in storage_proof_result.contracts_storage_proofs.into_iter().flatten() {
            let previous = proof_nodes.insert(node.node_hash, node.node.clone());
            if let Some(previous) = previous {
                // if there is a hash collision, the value should be the same
                assert!(previous == node.node);
            }
        }

        // for each contract we have a proof for, walk through the proof for all storage keys requested
        for contract_address in contract_storage.keys() {
            let storage_root = storage_roots
                .get(contract_address)
                .unwrap_or_else(|| panic!("no proof returned for contract {:x}", contract_address));

            let keys = contract_storage_keys.get(contract_address).unwrap();
            for key in keys {
                let path = verify_proof::<Pedersen>(storage_root, key, &proof_nodes)?;

                // should have at least two nodes assuming at least 2 values.
                assert!(path.len() >= keys.len().min(2));
            }
        }

        Ok(())
    }

    #[rstest::rstest]
    #[case(vec![
        (Felt::TWO, Felt::TWO)
    ])]
    #[case(vec![
        (Felt::from(5), Felt::from(55)),
        (Felt::from(6), Felt::from(66)),
        (Felt::from(7), Felt::from(77)),
    ])]
    #[case(vec![
        (Felt::from_hex_unchecked("0x0000000000000000000000000000000000000000000000000000000000000000"), Felt::from(11)),
        (Felt::from_hex_unchecked("0x0000000000000000000000000000000000000000000000000000000000000001"), Felt::from(22)),
        (Felt::from_hex_unchecked("0x0100000000000000000000000000000000000000000000000000000000000000"), Felt::from(33)),
        (Felt::from_hex_unchecked("0x0fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"), Felt::from(44)),
    ])]
    #[tokio::test]
    /// Tests the class trie, ensuring the validity of the proof and that it maches the global
    /// class trie root.
    async fn test_class_trie_proof(
        #[case] class_items: Vec<(Felt, Felt)>,
        #[with(1)] block: mp_block::MadaraMaybePendingBlock,
        rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, Starknet),
    ) -> Result<(), String> {
        use starknet_types_core::hash::Poseidon;

        let (_backend, starknet) = rpc_test_setup;

        let mut class_trie = starknet.backend.class_trie();
        let mut class_keys = Vec::new();

        // the class trie is just one MPT (unlike the contract storage MPT), we just insert k:v
        // pairs into it with a well-known identifier for the trie itself
        for (class_hash, value) in class_items {
            class_trie.insert(bonsai_identifier::CLASS, &class_hash.to_bytes_be().as_bits()[5..], &value).unwrap();

            class_keys.push(class_hash);
        }
        class_trie.commit(BasicId::new(1)).expect("failed to commit to class_trie");

        // create a dummy block to make get_storage_proof() happy
        // (it wants a block to exist for the requested chain height)
        starknet.backend.store_block(block, StateDiff::default(), vec![]).unwrap();

        let storage_proof_result =
            get_storage_proof(&starknet, BlockId::Tag(BlockTag::Latest), Some(class_keys.clone()), None, None).unwrap();

        let mut proof_nodes = HashMap::new();
        for node in storage_proof_result.classes_proof.into_iter() {
            proof_nodes.insert(node.node_hash, node.node);
        }

        for key in &class_keys {
            let path =
                verify_proof::<Poseidon>(&storage_proof_result.global_roots.classes_tree_root, key, &proof_nodes)?;

            // should have at least two nodes assuming at least 2 values.
            assert!(path.len() >= class_keys.len().min(2));
        }

        Ok(())
    }

    #[rstest::rstest]
    #[case(vec![
        (Felt::TWO, Felt::TWO)
    ])]
    #[case(vec![
        (Felt::from_hex_unchecked("0x0000000000000000000000000000000000000000000000000000000000000000"), Felt::from(11)),
        (Felt::from_hex_unchecked("0x0000000000000000000000000000000000000000000000000000000000000001"), Felt::from(22)),
        (Felt::from_hex_unchecked("0x0100000000000000000000000000000000000000000000000000000000000000"), Felt::from(33)),
        (Felt::from_hex_unchecked("0x0fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"), Felt::from(44)),
    ])]
    #[tokio::test]
    /// Tests the contract trie, ensuring the validity of the proof and that it maches the global
    /// contract trie root.
    async fn test_contract_trie_proof(
        #[case] contract_items: Vec<(Felt, Felt)>,
        #[with(1)] block: mp_block::MadaraMaybePendingBlock,
        rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, Starknet),
    ) -> Result<(), String> {
        let (_backend, starknet) = rpc_test_setup;

        let mut contract_trie = starknet.backend.contract_trie();
        let mut contract_addresses = Vec::new();

        // the contract trie is just one MPT (unlike the contract-storage MPT), we just insert k:v
        // pairs into it with a well-known identifier for the trie itself
        for (contract_address, value) in contract_items {
            contract_trie
                .insert(bonsai_identifier::CONTRACT, &contract_address.to_bytes_be().as_bits()[5..], &value)
                .unwrap();

            contract_addresses.push(contract_address);
        }
        contract_trie.commit(BasicId::new(1)).expect("failed to commit to contract_trie");

        // create a dummy block to make get_storage_proof() happy
        // (it wants a block to exist for the requested chain height)
        starknet.backend.store_block(block, StateDiff::default(), vec![]).unwrap();

        let storage_proof_result =
            get_storage_proof(&starknet, BlockId::Tag(BlockTag::Latest), None, Some(contract_addresses.clone()), None)
                .unwrap();

        let mut proof_nodes = HashMap::new();
        for node in storage_proof_result.contracts_proof.nodes.into_iter() {
            proof_nodes.insert(node.node_hash, node.node);
        }

        for key in &contract_addresses {
            let path =
                verify_proof::<Pedersen>(&storage_proof_result.global_roots.contracts_tree_root, key, &proof_nodes)?;

            // should have at least two nodes assuming at least 2 values.
            assert!(path.len() >= contract_addresses.len().min(2));
        }

        Ok(())
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
        H::hash(&child_hash, felt_path) + length
    }

    /// Verifies a proof from `commitment` (the root MPT hash) to the leaf identified by `path`.
    ///
    /// This algorithm looks up each node by hash, expecting `proof_nodes` to contain either a
    /// Binary node or an Edge node for each, starting with `commitment`. For each node
    /// encountered, it does the following:
    ///  * verify the node's hash (by hashing the node)
    ///  * (for binary node): continue left or right to the next child
    ///  * (for edge node): verify the edge's path matches, then jump to the end of the edge
    ///
    /// Additionally, the algorithm ensures that we got to the bottom of the tree (total path
    /// traveled should be 251).
    ///
    /// The algorithm does not attempt to verify the leaf nodes themselves.
    ///
    /// The proof_nodes is essentially a preimage-lookup table, and may contain proof nodes that are
    /// irrelevant to the given path.
    pub fn verify_proof<H: StarkHash>(
        commitment: &Felt,
        path: &Felt,
        proof_nodes: &HashMap<Felt, MerkleNode>,
    ) -> Result<Vec<MerkleNode>, String> {
        let start = 5; // 256 minus 251
        let mut index = start;
        let path_bits: BitVec<_, Msb0> = BitVec::from_slice(&path.to_bytes_be());

        let mut next_node_hash = commitment;
        let mut ordered_proof = Vec::new();
        loop {
            let node = proof_nodes
                .get(next_node_hash)
                .ok_or(format!("proof did not contain preimage for node 0x{:x} (index: {})", next_node_hash, index))?;
            match node {
                MerkleNode::Binary { left, right } => {
                    let actual_node_hash = hash_binary_node::<H>(*left, *right);
                    if &actual_node_hash != next_node_hash {
                        return Err(format!(
                            "incorrect binary node hash (expected 0x{:x}, but got 0x{:x})",
                            next_node_hash, actual_node_hash
                        ));
                    }
                    next_node_hash = if path_bits[index] { right } else { left };
                    index += 1;
                }
                MerkleNode::Edge { child, path, length } => {
                    let relevant_path = &path_bits[index..index + length];

                    let node_path_bits: BitVec<_, Msb0> = BitVec::from_slice(&path.to_bytes_be());
                    let relevant_node_path = &node_path_bits[256 - *length..];

                    if relevant_path != relevant_node_path {
                        return Err(format!(
                            "incorrect edge path (expected {:?}, but got {:?})",
                            relevant_path, relevant_node_path
                        ));
                    }

                    let actual_node_hash = hash_edge_node::<H>(path, *length, *child);
                    if &actual_node_hash != next_node_hash {
                        return Err(format!(
                            "incorrect edge node hash (expected {:x}, but got {:x})",
                            next_node_hash, actual_node_hash
                        ));
                    }
                    next_node_hash = child;
                    index += length;
                }
            }

            ordered_proof.push(node.clone());

            if index > 256 {
                return Err(format!("invalid proof, path too long ({})", (index - start)));
            }
            if index == 256 {
                break;
            }
        }

        Ok(ordered_proof)
    }
}
