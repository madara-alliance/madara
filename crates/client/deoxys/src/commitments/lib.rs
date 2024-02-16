use std::sync::Arc;

use blockifier::state::cached_state::CommitmentStateDiff;
use indexmap::IndexMap;
use mc_db::bonsai_db::BonsaiDb;
use mc_db::BonsaiDbError;
use mp_block::state_update::StateUpdateWrapper;
use mp_felt::Felt252Wrapper;
use mp_hashers::poseidon::PoseidonHasher;
use mp_hashers::HasherT;
use mp_transactions::Transaction;
use sp_runtime::traits::Block as BlockT;
use starknet_api::api_core::{ClassHash, CompiledClassHash, ContractAddress, Nonce};
use starknet_api::hash::StarkFelt;
use starknet_api::state::StorageKey;
use starknet_api::transaction::Event;

use super::classes::{get_class_trie_root, update_class_trie};
use super::contracts::{get_contract_trie_root, update_contract_trie, ContractLeafParams};
use super::events::memory_event_commitment;
use super::transactions::memory_transaction_commitment;

/// Calculate the transaction commitment, the event commitment and the event count.
///
/// # Arguments
///
/// * `transactions` - The transactions of the block
///
/// # Returns
///
/// The transaction commitment, the event commitment and the event count.
pub fn calculate_commitments(
    transactions: &[Transaction],
    events: &[Event],
    chain_id: Felt252Wrapper,
    block_number: u64,
) -> (Felt252Wrapper, Felt252Wrapper) {
    (
        memory_transaction_commitment(transactions, chain_id, block_number)
            .expect("Failed to calculate transaction commitment"),
        memory_event_commitment(events).expect("Failed to calculate event commitment"),
    )
}

pub fn build_commitment_state_diff(state_update_wrapper: StateUpdateWrapper) -> CommitmentStateDiff {
    let mut commitment_state_diff = CommitmentStateDiff {
        address_to_class_hash: IndexMap::new(),
        address_to_nonce: IndexMap::new(),
        storage_updates: IndexMap::new(),
        class_hash_to_compiled_class_hash: IndexMap::new(),
    };

    for deployed_contract in state_update_wrapper.state_diff.deployed_contracts.iter() {
        let address = ContractAddress::from(deployed_contract.address.clone());
        let class_hash = ClassHash::from(deployed_contract.class_hash.clone());
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
/// It combines the roots of two binary Merkle-Patricia tries of height 251 using Poseidon hasher.
///
/// # Arguments
///
/// * `contracts_trie_root` - The root of the contracts trie.
/// * `classes_trie_root` - The root of the classes trie.
///
/// # Returns
///
/// The state commitment as a `StateCommitment`.
pub fn calculate_state_root<H: HasherT>(
    contracts_trie_root: Felt252Wrapper,
    classes_trie_root: Felt252Wrapper,
) -> Felt252Wrapper
where
    H: HasherT,
{
    let starknet_state_prefix = Felt252Wrapper::try_from("STARKNET_STATE_V0".as_bytes()).unwrap();

    let state_commitment_hash =
        H::compute_hash_on_elements(&[starknet_state_prefix.0, contracts_trie_root.0, classes_trie_root.0]);

    state_commitment_hash.into()
}

pub fn update_state_root<B: BlockT>(
    csd: CommitmentStateDiff,
    bonsai_db: &Arc<BonsaiDb<B>>,
) -> Result<Felt252Wrapper, BonsaiDbError> {
    let mut contract_trie_root = Felt252Wrapper::default();
    let mut class_trie_root = Felt252Wrapper::default();

    for (address, class_hash) in csd.address_to_class_hash.iter() {
        let storage_root = Felt252Wrapper::default();
        let nonce = csd.address_to_nonce.get(address).unwrap_or(&Felt252Wrapper::default().into()).clone();

        let contract_leaf_params = ContractLeafParams { class_hash: class_hash.clone().into(), storage_root, nonce: nonce.into() };

        contract_trie_root = update_contract_trie(address.clone().into(), contract_leaf_params, bonsai_db)?;
    }

    for (class_hash, compiled_class_hash) in csd.class_hash_to_compiled_class_hash.iter() {
        class_trie_root = update_class_trie(class_hash.clone().into(), compiled_class_hash.clone().into(), bonsai_db)?;
    }

    let state_root = calculate_state_root::<PoseidonHasher>(contract_trie_root, class_trie_root);

    Ok(state_root)
}

pub fn state_root<B, H>(bonsai_db: &Arc<BonsaiDb<B>>) -> Felt252Wrapper
where
    B: BlockT,
    H: HasherT,
{
    let contract_trie_root = get_contract_trie_root(bonsai_db).expect("Failed to get contract trie root");
    let class_trie_root = get_class_trie_root(bonsai_db).expect("Failed to get class trie root");

    calculate_state_root::<H>(contract_trie_root, class_trie_root)
}
