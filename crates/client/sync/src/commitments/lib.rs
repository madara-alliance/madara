use std::sync::{Arc, Mutex, MutexGuard};

use bitvec::order::Msb0;
use bitvec::vec::BitVec;
use bitvec::view::BitView;
use blockifier::state::cached_state::CommitmentStateDiff;
use bonsai_trie::id::BasicId;
use bonsai_trie::{BonsaiStorage, BonsaiStorageError};
use indexmap::IndexMap;
use mc_db::bonsai_db::BonsaiDb;
use mc_db::BonsaiDbError;
use mc_storage::OverrideHandle;
use mp_block::state_update::StateUpdateWrapper;
use mp_felt::Felt252Wrapper;
use mp_hashers::pedersen::PedersenHasher;
use mp_hashers::poseidon::PoseidonHasher;
use mp_hashers::HasherT;
use mp_storage::StarknetStorageSchemaVersion::Undefined;
use mp_transactions::Transaction;
use sp_core::hexdisplay::AsBytesRef;
use sp_core::H256;
use sp_runtime::generic::{Block, Header};
use sp_runtime::traits::{BlakeTwo256, Block as BlockT};
use sp_runtime::OpaqueExtrinsic;
use starknet_api::api_core::{ClassHash, CompiledClassHash, ContractAddress, Nonce};
use starknet_api::hash::StarkFelt;
use starknet_api::state::StorageKey;
use starknet_api::transaction::Event;
use starknet_types_core::hash::{Pedersen, Poseidon};
use tokio::join;

use super::classes::calculate_class_commitment_leaf_hash;
use super::contracts::{identifier, update_storage_trie, ContractLeafParams};
use super::events::memory_event_commitment;
use super::transactions::memory_transaction_commitment;
use crate::commitments::contracts::calculate_contract_state_leaf_hash;
use crate::utils::constant::bonsai_identifier;

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
        let address = ContractAddress::from(deployed_contract.address);
        let class_hash = if address == ContractAddress::from(Felt252Wrapper::ONE) {
            // System contracts doesnt have class hashes
            ClassHash::from(Felt252Wrapper::ZERO)
        } else {
            ClassHash::from(deployed_contract.class_hash)
        };
        commitment_state_diff.address_to_class_hash.insert(address, class_hash);
    }

    for (address, nonce) in state_update_wrapper.state_diff.nonces.iter() {
        let contract_address = ContractAddress::from(*address);
        let nonce_value = Nonce::from(*nonce);
        commitment_state_diff.address_to_nonce.insert(contract_address, nonce_value);
    }

    for (address, storage_diffs) in state_update_wrapper.state_diff.storage_diffs.iter() {
        let contract_address = ContractAddress::from(*address);
        let mut storage_map = IndexMap::new();
        for storage_diff in storage_diffs.iter() {
            let key = StorageKey::from(storage_diff.key);
            let value = StarkFelt::from(storage_diff.value);
            storage_map.insert(key, value);
        }
        commitment_state_diff.storage_updates.insert(contract_address, storage_map);
    }

    for declared_class in state_update_wrapper.state_diff.declared_classes.iter() {
        let class_hash = ClassHash::from(declared_class.class_hash);
        let compiled_class_hash = CompiledClassHash::from(declared_class.compiled_class_hash);
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
    overrides: Arc<OverrideHandle<Block<Header<u32, BlakeTwo256>, OpaqueExtrinsic>>>,
    bonsai_contract: Arc<Mutex<BonsaiStorage<BasicId, BonsaiDb<B>, Pedersen>>>,
    bonsai_contract_storage: Arc<Mutex<BonsaiStorage<BasicId, BonsaiDb<B>, Pedersen>>>,
    bonsai_class: Arc<Mutex<BonsaiStorage<BasicId, BonsaiDb<B>, Poseidon>>>,
    block_number: u64,
    substrate_block_hash: Option<H256>,
) -> Felt252Wrapper {
    // Update contract and its storage tries
    let contract_trie_root = contract_trie_root(
        &csd,
        overrides,
        bonsai_contract.lock().unwrap(),
        bonsai_contract_storage,
        block_number,
        substrate_block_hash,
    )
    .expect("Failed to compute contract root");

    // Update class trie
    let class_trie_root =
        class_trie_root(&csd, bonsai_class.lock().unwrap(), block_number).expect("Failed to compute class root");

    calculate_state_root::<PoseidonHasher>(contract_trie_root, class_trie_root)
}

/// Calculates the contract trie root
///
/// # Arguments
///
/// * `csd`             - Commitment state diff for the current block.
/// * `overrides`       - Deoxys storage override for accessing the Substrate db.
/// * `bonsai_contract` - Bonsai db used to store contract hashes.
/// * `block_number`    - The current block number.
///
/// # Returns
///
/// The contract root.
fn contract_trie_root<B: BlockT>(
    csd: &CommitmentStateDiff,
    overrides: Arc<OverrideHandle<Block<Header<u32, BlakeTwo256>, OpaqueExtrinsic>>>,
    mut bonsai_contract: MutexGuard<BonsaiStorage<BasicId, BonsaiDb<B>, Pedersen>>,
    bonsai_contract_storage: Arc<Mutex<BonsaiStorage<BasicId, BonsaiDb<B>, Pedersen>>>,
    block_number: u64,
    maybe_block_hash: Option<H256>,
) -> Result<Felt252Wrapper, BonsaiStorageError<BonsaiDbError>> {
    let identifier = bonsai_identifier::CONTRACT;
    bonsai_contract.init_tree(identifier)?;

    // First we insert the contract storage changes
    // TODO: @cchudant parallelize this loop
    for (contract_address, updates) in csd.storage_updates.iter() {
        update_storage_trie(contract_address, updates, &bonsai_contract_storage);
    }

    // Then we commit them
    bonsai_contract_storage.lock().unwrap().commit(BasicId::new(block_number))?;

    // Then we compute the leaf hashes retrieving the corresponding storage root
    // TODO: @cchudant parallelize this loop
    for contract_address in csd.storage_updates.iter() {
        let class_commitment_leaf_hash =
            contract_state_leaf_hash(csd, &overrides, contract_address.0, maybe_block_hash, &bonsai_contract_storage)?;

        let key = key(contract_address.0.0.0);
        bonsai_contract.insert(identifier, &key, &class_commitment_leaf_hash.into())?;
    }

    bonsai_contract.commit(BasicId::new(block_number))?;
    Ok(bonsai_contract.root_hash(identifier)?.into())
}

fn contract_state_leaf_hash<B: BlockT>(
    csd: &CommitmentStateDiff,
    overrides: &OverrideHandle<Block<Header<u32, BlakeTwo256>, OpaqueExtrinsic>>,
    contract_address: &ContractAddress,
    maybe_block_hash: Option<H256>,
    bonsai_contract_storage: &Arc<Mutex<BonsaiStorage<BasicId, BonsaiDb<B>, Pedersen>>>,
) -> Result<Felt252Wrapper, BonsaiStorageError<BonsaiDbError>> {
    let identifier = identifier(contract_address);
    let storage_root =
        bonsai_contract_storage.lock().unwrap().root_hash(identifier).expect("Failed to get root hash").into();

    let nonce =
        Felt252Wrapper::from(*csd.address_to_nonce.get(contract_address).unwrap_or(&Felt252Wrapper::ZERO.into()));

    let class_hash = class_hash(csd, overrides, contract_address, maybe_block_hash);
    let contract_leaf_params = ContractLeafParams { class_hash, storage_root, nonce };
    Ok(calculate_contract_state_leaf_hash::<PedersenHasher>(contract_leaf_params))
}

fn class_hash(
    csd: &CommitmentStateDiff,
    overrides: &OverrideHandle<Block<Header<u32, BlakeTwo256>, OpaqueExtrinsic>>,
    contract_address: &ContractAddress,
    maybe_block_hash: Option<H256>,
) -> Felt252Wrapper {
    let class_hash = match csd.address_to_class_hash.get(contract_address) {
        Some(class_hash) => *class_hash,
        None => match maybe_block_hash {
            Some(block_hash) => overrides
                .for_schema_version(&Undefined)
                .contract_class_hash_by_address(block_hash, *contract_address)
                .unwrap(),
            None => unreachable!(),
        },
    };

    Felt252Wrapper::from(class_hash)
}

/// Calculates the class trie root
///
/// # Arguments
///
/// * `csd`          - Commitment state diff for the current block.
/// * `bonsai_class` - Bonsai db used to store class hashes.
/// * `block_number` - The current block number.
///
/// # Returns
///
/// The class root.
fn class_trie_root<B: BlockT>(
    csd: &CommitmentStateDiff,
    mut bonsai_class: MutexGuard<BonsaiStorage<BasicId, BonsaiDb<B>, Poseidon>>,
    block_number: u64,
) -> Result<Felt252Wrapper, BonsaiStorageError<BonsaiDbError>> {
    let identifier = bonsai_identifier::CLASS;
    bonsai_class.init_tree(identifier)?;

    // TODO: @cchudant parallelize this loop
    for (class_hash, compiled_class_hash) in csd.class_hash_to_compiled_class_hash.iter() {
        let class_commitment_leaf_hash =
            calculate_class_commitment_leaf_hash::<PoseidonHasher>(Felt252Wrapper::from(compiled_class_hash.0));
        let key = key(class_hash.0);
        bonsai_class.insert(identifier, key.as_bitslice(), &class_commitment_leaf_hash.into())?;
    }

    bonsai_class.commit(BasicId::new(block_number))?;
    Ok(bonsai_class.root_hash(identifier)?.into())
}

/// Compute the bonsai storage key
///
/// # Arguments
///
/// * `felt` - Value to use as storage key.
///
/// # Returns
///
/// Storage key
pub fn key(felt: StarkFelt) -> BitVec<u8, Msb0> {
    felt.0.view_bits()[5..].to_owned()
}
