use std::sync::Arc;

use bitvec::view::BitView;
use blockifier::state::cached_state::CommitmentStateDiff;
use bonsai_trie::id::BasicId;
use indexmap::IndexMap;
use mc_db::bonsai_db::BonsaiConfigs;
use mc_db::BonsaiDbError;
use mc_storage::OverrideHandle;
use mp_block::state_update::StateUpdateWrapper;
use mp_felt::Felt252Wrapper;
use mp_hashers::pedersen::PedersenHasher;
use mp_hashers::poseidon::PoseidonHasher;
use mp_hashers::HasherT;
use mp_transactions::Transaction;
use sp_core::H256;
use sp_runtime::generic::{Block, Header};
use sp_runtime::traits::{BlakeTwo256, Block as BlockT};
use sp_runtime::OpaqueExtrinsic;
use starknet_api::api_core::{ClassHash, CompiledClassHash, ContractAddress, Nonce};
use starknet_api::hash::StarkFelt;
use starknet_api::state::StorageKey;
use starknet_api::transaction::Event;
use tokio::join;

use super::classes::calculate_class_commitment_leaf_hash;
use super::contracts::{update_storage_trie, ContractLeafParams};
use super::events::memory_event_commitment;
use super::transactions::memory_transaction_commitment;
use crate::commitments::contracts::calculate_contract_state_leaf_hash;

/// Calculate the transaction and event commitment.
///
/// # Arguments
///
/// * `transactions` - The transactions of the block
/// * `events` - The events of the block
/// * `chain_id` - The current chain id
/// * `block_number` - The current block number
///
/// # Returns
///
/// The transaction and the event commitment as `Felt252Wrapper`.
pub async fn calculate_commitments(
    transactions: &[Transaction],
    events: &[Event],
    chain_id: Felt252Wrapper,
    block_number: u64,
) -> (Felt252Wrapper, Felt252Wrapper) {
    let (commitment_tx, commitment_event) =
        join!(memory_transaction_commitment(transactions, chain_id, block_number), memory_event_commitment(events));

    (
        commitment_tx.expect("Failed to calculate transaction commitment"),
        commitment_event.expect("Failed to calculate event commitment"),
    )
}

/// Builds a `CommitmentStateDiff` from the `StateUpdateWrapper`.
///
/// # Arguments
///
/// * `StateUpdateWrapper` - The last state update fetched and formated.
///
/// # Returns
///
/// The commitment state diff as a `CommitmentStateDiff`.
pub fn build_commitment_state_diff(state_update_wrapper: StateUpdateWrapper) -> CommitmentStateDiff {
    let mut commitment_state_diff = CommitmentStateDiff {
        address_to_class_hash: IndexMap::new(),
        address_to_nonce: IndexMap::new(),
        storage_updates: IndexMap::new(),
        class_hash_to_compiled_class_hash: IndexMap::new(),
    };

    for deployed_contract in state_update_wrapper.state_diff.deployed_contracts.iter() {
        let address = ContractAddress::from(deployed_contract.address.clone());
        let class_hash = if address == ContractAddress::from(Felt252Wrapper::ONE) {
            // System contracts doesnt have class hashes
            ClassHash::from(Felt252Wrapper::ZERO)
        } else {
            ClassHash::from(deployed_contract.class_hash.clone())
        };
        commitment_state_diff.address_to_class_hash.insert(address, class_hash);
    }

    for (address, nonce) in state_update_wrapper.state_diff.nonces.iter() {
        let contract_address = ContractAddress::from(address.clone());
        let nonce_value = Nonce::from(nonce.clone());
        commitment_state_diff.address_to_nonce.insert(contract_address, nonce_value);
    }

    for (address, storage_diffs) in state_update_wrapper.state_diff.storage_diffs.iter() {
        let contract_address = ContractAddress::from(address.clone());
        let mut storage_map = IndexMap::new();
        for storage_diff in storage_diffs.iter() {
            let key = StorageKey::from(storage_diff.key.clone());
            let value = StarkFelt::from(storage_diff.value.clone());
            storage_map.insert(key, value);
        }
        commitment_state_diff.storage_updates.insert(contract_address, storage_map);
    }

    for declared_class in state_update_wrapper.state_diff.declared_classes.iter() {
        let class_hash = ClassHash::from(declared_class.class_hash.clone());
        let compiled_class_hash = CompiledClassHash::from(declared_class.compiled_class_hash.clone());
        commitment_state_diff.class_hash_to_compiled_class_hash.insert(class_hash, compiled_class_hash);
    }

    commitment_state_diff
}

/// Calculate state commitment hash value.
///
/// The state commitment is the digest that uniquely (up to hash collisions) encodes the state.
/// It combines the roots of two binary Merkle-Patricia tries of height 251 using Poseidon/Pedersen
/// hashers.
///
/// # Arguments
///
/// * `contracts_trie_root` - The root of the contracts trie.
/// * `classes_trie_root` - The root of the classes trie.
///
/// # Returns
///
/// The state commitment as a `Felt252Wrapper`.
pub fn calculate_state_root<H: HasherT>(
    contracts_trie_root: Felt252Wrapper,
    classes_trie_root: Felt252Wrapper,
) -> Felt252Wrapper
where
    H: HasherT,
{
    let starknet_state_prefix = Felt252Wrapper::try_from("STARKNET_STATE_V0".as_bytes()).unwrap();

    if classes_trie_root == Felt252Wrapper::ZERO {
        contracts_trie_root
    } else {
        let state_commitment_hash =
            H::compute_hash_on_elements(&[starknet_state_prefix.0, contracts_trie_root.0, classes_trie_root.0]);

        state_commitment_hash.into()
    }
}

/// Update the state commitment hash value.
///
/// The state commitment is the digest that uniquely (up to hash collisions) encodes the state.
/// It combines the roots of two binary Merkle-Patricia tries of height 251 using Poseidon/Pedersen
/// hashers.
///
/// # Arguments
///
/// * `CommitmentStateDiff` - The commitment state diff inducing unprocessed state changes.
/// * `BonsaiDb` - The database responsible for storing computing the state tries.
///
/// # Returns
///
/// The updated state root as a `Felt252Wrapper`.
pub fn update_state_root<B: BlockT>(
    csd: CommitmentStateDiff,
    bonsai_dbs: &mut BonsaiConfigs<B>,
    block_number: u64,
    overrides: Arc<OverrideHandle<Block<Header<u32, BlakeTwo256>, OpaqueExtrinsic>>>,
    substrate_block_hash: Option<H256>,
) -> Result<Felt252Wrapper, BonsaiDbError> {
    // Update contract and its storage tries
    for (contract_address, class_hash) in csd.address_to_class_hash.iter() {
        let storage_root = update_storage_trie(contract_address, csd.clone(), overrides.clone(), substrate_block_hash)
            .expect("Failed to update storage trie");
        let nonce = csd.address_to_nonce.get(contract_address).unwrap_or(&Felt252Wrapper::ZERO.into()).clone();

        let contract_leaf_params =
            ContractLeafParams { class_hash: class_hash.clone().into(), storage_root, nonce: nonce.into() };
        let class_commitment_leaf_hash = calculate_contract_state_leaf_hash::<PedersenHasher>(contract_leaf_params);

        let key = contract_address.0.0.0.view_bits()[5..].to_owned();
        bonsai_dbs
            .contract
            .insert(&key, &class_commitment_leaf_hash.into())
            .expect("Failed to insert into bonsai storage");
    }

    bonsai_dbs.contract.commit(BasicId::new(block_number)).expect("Failed to commit to bonsai storage");
    let contract_trie_root = bonsai_dbs.contract.root_hash().expect("Failed to get root hash").into();

    // Update class trie
    for (class_hash, compiled_class_hash) in csd.class_hash_to_compiled_class_hash.iter() {
        let class_commitment_leaf_hash =
            calculate_class_commitment_leaf_hash::<PoseidonHasher>(Felt252Wrapper::from(compiled_class_hash.0));
        let key = class_hash.0.0.view_bits()[5..].to_owned();
        bonsai_dbs
            .class
            .insert(key.as_bitslice(), &class_commitment_leaf_hash.into())
            .expect("Failed to insert into bonsai storage");
    }

    bonsai_dbs.class.commit(BasicId::new(block_number)).expect("Failed to commit to bonsai storage");
    let class_trie_root = bonsai_dbs.class.root_hash().expect("Failed to get root hash").into();

    let state_root = calculate_state_root::<PoseidonHasher>(contract_trie_root, class_trie_root);
    Ok(state_root)
}
