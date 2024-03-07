use std::sync::Arc;

use blockifier::state::cached_state::CommitmentStateDiff;
use indexmap::IndexMap;
use mc_db::bonsai_db::BonsaiDb;
use mc_db::BonsaiDbs;
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
use tokio::join;
use tokio::task::{spawn_blocking, JoinSet};

use super::classes::{get_class_trie_root, update_class_trie};
use super::contracts::{get_contract_trie_root, update_contract_trie, update_storage_trie, ContractLeafParams};
use super::events::memory_event_commitment;
use super::transactions::memory_transaction_commitment;

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

    let state_commitment_hash =
        H::compute_hash_on_elements(&[starknet_state_prefix.0, contracts_trie_root.0, classes_trie_root.0]);

    state_commitment_hash.into()
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
pub async fn update_state_root<B: BlockT>(
    csd: CommitmentStateDiff,
    bonsai_dbs: BonsaiDbs<B>,
) -> anyhow::Result<Felt252Wrapper> {
    let arc_csd = Arc::new(csd);
    let arc_bonsai_dbs = Arc::new(bonsai_dbs);

    let contract_trie_root = contract_trie_root(Arc::clone(&arc_csd), Arc::clone(&arc_bonsai_dbs)).await?;

    let class_trie_root = class_trie_root(Arc::clone(&arc_csd), Arc::clone(&arc_bonsai_dbs))?;

    let state_root = calculate_state_root::<PoseidonHasher>(contract_trie_root, class_trie_root);

    Ok(state_root)
}

async fn contract_trie_root<B: BlockT>(
    csd: Arc<CommitmentStateDiff>,
    bonsai_dbs: Arc<BonsaiDbs<B>>,
) -> anyhow::Result<Felt252Wrapper> {
    // Risk of starving the thread pool (execution over 1s in some cases), must be run in a
    // blocking-safe thread. Main bottleneck is still calling `commit` on the Bonsai db.
    let mut task_set = spawn_blocking(move || {
        let mut task_set = JoinSet::new();

        csd.address_to_class_hash.iter().for_each(|(contract_address, class_hash)| {
            let csd_clone = Arc::clone(&csd);
            let bonsai_dbs_clone = Arc::clone(&bonsai_dbs);

            task_set.spawn(contract_trie_root_loop(csd_clone, bonsai_dbs_clone, *contract_address, *class_hash));
        });

        task_set
    })
    .await?;

    // The order in which contract trie roots are waited for is not important since each call to
    // `update_contract_trie` in `contract_trie_root` mutates the Deoxys db.
    let mut contract_trie_root = Felt252Wrapper::ZERO;
    while let Some(res) = task_set.join_next().await {
        contract_trie_root = match res? {
            Ok(trie_root) => trie_root,
            Err(e) => {
                task_set.abort_all();
                return Err(e);
            }
        }
    }

    Ok(contract_trie_root)
}

async fn contract_trie_root_loop<B: BlockT>(
    csd: Arc<CommitmentStateDiff>,
    bonsai_dbs: Arc<BonsaiDbs<B>>,
    contract_address: ContractAddress,
    class_hash: ClassHash,
) -> anyhow::Result<Felt252Wrapper> {
    let storage_root =
        update_storage_trie(&contract_address, &csd, &bonsai_dbs.storage).expect("Failed to update storage trie");
    let nonce = *csd.address_to_nonce.get(&contract_address).unwrap_or(&Felt252Wrapper::default().into());

    let contract_leaf_params = ContractLeafParams { class_hash: class_hash.into(), storage_root, nonce: nonce.into() };

    update_contract_trie(contract_address.into(), contract_leaf_params, &bonsai_dbs.contract)
}

fn class_trie_root<B: BlockT>(
    csd: Arc<CommitmentStateDiff>,
    bonsai_dbs: Arc<BonsaiDbs<B>>,
) -> anyhow::Result<Felt252Wrapper> {
    let mut class_trie_root = Felt252Wrapper::default();

    // Based on benchmarks the execution cost of computing the class tried root is negligible
    // compared to the contract trie root. It is likely that parallelizing this would yield no
    // observalble benefits.
    for (class_hash, compiled_class_hash) in csd.class_hash_to_compiled_class_hash.iter() {
        class_trie_root = update_class_trie((*class_hash).into(), (*compiled_class_hash).into(), &bonsai_dbs.class)?;
    }

    Ok(class_trie_root)
}

/// Retrieves and compute the actual state root.
///
/// The state commitment is the digest that uniquely (up to hash collisions) encodes the state.
/// It combines the roots of two binary Merkle-Patricia tries of height 251 using Poseidon/Pedersen
/// hasher.
///
/// # Arguments
///
/// * `BonsaiDb` - The database responsible for storing computing the state tries.
///
/// # Returns
///
/// The actual state root as a `Felt252Wrapper`.
pub fn state_root<B, H>(bonsai_db: &Arc<BonsaiDb<B>>) -> Felt252Wrapper
where
    B: BlockT,
    H: HasherT,
{
    let contract_trie_root = get_contract_trie_root(bonsai_db).expect("Failed to get contract trie root");
    let class_trie_root = get_class_trie_root(bonsai_db).expect("Failed to get class trie root");

    calculate_state_root::<H>(contract_trie_root, class_trie_root)
}
