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
use starknet_api::api_core::{ClassHash, CompiledClassHash, ContractAddress, Nonce, PatriciaKey};
use starknet_api::hash::StarkFelt;
use starknet_api::state::{StorageKey, ThinStateDiff};
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
    println!("contract_trie_root: {:?}", contract_trie_root);
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

            task_set.spawn(contract_trie_root_loop(
                csd_clone,
                bonsai_dbs_clone,
                contract_address.clone(),
                class_hash.clone(),
            ));
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
    let nonce = csd.address_to_nonce.get(&contract_address).unwrap_or(&Felt252Wrapper::default().into()).clone();

    let contract_leaf_params =
        ContractLeafParams { class_hash: class_hash.clone().into(), storage_root, nonce: nonce.into() };

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
        class_trie_root =
            update_class_trie(class_hash.clone().into(), compiled_class_hash.clone().into(), &bonsai_dbs.class)?;
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::str::FromStr;
    use std::string;

    use starknet_core::types::BlockId;
    use starknet_ff::FieldElement;
    use starknet_providers::sequencer::models::state_update::{DeployedContract, StateDiff, StorageDiff};
    use starknet_providers::sequencer::models::StateUpdate;
    use starknet_providers::SequencerGatewayProvider;
    use url::Url;

    use super::*;

    pub fn feltor(input: &str) -> FieldElement {
        FieldElement::from_hex_be(input).unwrap()
    }

    pub fn fetch_raw_genesis_state_update() -> StateUpdate {
        StateUpdate {
            block_hash: Some(feltor("0x47c3637b57c2b079b93c61539950c17e868a28f46cdef28f88521067f21e943")),
            new_root: Some(feltor("0x21870ba80540e7831fb21c591ee93481f5ae1bb71ff85a86ddd465be4eddee6")),
            old_root: feltor("0x0"),
            state_diff: StateDiff {
                storage_diffs: vec![
                    (feltor("0x20cfa74ee3564b4cd5435cdace0f9c4d43b939620e4a0bb5076105df0a626c6"), vec![
                        StorageDiff { key: feltor("0x5"), value: feltor("0x22b") },
                        StorageDiff { key: feltor("0x313ad57fdf765addc71329abf8d74ac2bce6d46da8c2b9b82255a5076620300"), value: feltor("0x4e7e989d58a17cd279eca440c5eaa829efb6f9967aaad89022acbe644c39b36") },
                        StorageDiff { key: feltor("0x313ad57fdf765addc71329abf8d74ac2bce6d46da8c2b9b82255a5076620301"), value: feltor("0x453ae0c9610197b18b13645c44d3d0a407083d96562e8752aab3fab616cecb0") },
                        StorageDiff { key: feltor("0x5aee31408163292105d875070f98cb48275b8c87e80380b78d30647e05854d5"), value: feltor("0x7e5") },
                        StorageDiff { key: feltor("0x6cf6c2f36d36b08e591e4489e92ca882bb67b9c39a3afccf011972a8de467f0"), value: feltor("0x7ab344d88124307c07b56f6c59c12f4543e9c96398727854a322dea82c73240") },
                    ]),
                    (feltor("0x31c887d82502ceb218c06ebb46198da3f7b92864a8223746bc836dda3e34b52"), vec![
                        StorageDiff { key: feltor("0xdf28e613c065616a2e79ca72f9c1908e17b8c913972a9993da77588dc9cae9"), value: feltor("0x1432126ac23c7028200e443169c2286f99cdb5a7bf22e607bcd724efa059040") },
                        StorageDiff { key: feltor("0x5f750dc13ed239fa6fc43ff6e10ae9125a33bd05ec034fc3bb4dd168df3505f"), value: feltor("0x7c7") },
                    ]),
                    (feltor("0x31c9cdb9b00cb35cf31c05855c0ec3ecf6f7952a1ce6e3c53c3455fcd75a280"), vec![
                        StorageDiff { key: feltor("0x5"), value: feltor("0x65") },
                        StorageDiff { key: feltor("0xcfc2e2866fd08bfb4ac73b70e0c136e326ae18fc797a2c090c8811c695577e"), value: feltor("0x5f1dd5a5aef88e0498eeca4e7b2ea0fa7110608c11531278742f0b5499af4b3") },
                        StorageDiff { key: feltor("0x5aee31408163292105d875070f98cb48275b8c87e80380b78d30647e05854d5"), value: feltor("0x7c7") },
                        StorageDiff { key: feltor("0x5fac6815fddf6af1ca5e592359862ede14f171e1544fd9e792288164097c35d"), value: feltor("0x299e2f4b5a873e95e65eb03d31e532ea2cde43b498b50cd3161145db5542a5") },
                        StorageDiff { key: feltor("0x5fac6815fddf6af1ca5e592359862ede14f171e1544fd9e792288164097c35e"), value: feltor("0x3d6897cf23da3bf4fd35cc7a43ccaf7c5eaf8f7c5b9031ac9b09a929204175f") },
                    ]),
                    (feltor("0x6ee3440b08a9c805305449ec7f7003f27e9f7e287b83610952ec36bdc5a6bae"), vec![
                        StorageDiff { key: feltor("0x1e2cd4b3588e8f6f9c4e89fb0e293bf92018c96d7a93ee367d29a284223b6ff"), value: feltor("0x71d1e9d188c784a0bde95c1d508877a0d93e9102b37213d1e13f3ebc54a7751") },
                        StorageDiff { key: feltor("0x449908c349e90f81ab13042b1e49dc251eb6e3e51092d9a40f86859f7f415b0"), value: feltor("0x6cb6104279e754967a721b52bcf5be525fdc11fa6db6ef5c3a4db832acf7804") },
                        StorageDiff { key: feltor("0x48cba68d4e86764105adcdcf641ab67b581a55a4f367203647549c8bf1feea2"), value: feltor("0x362d24a3b030998ac75e838955dfee19ec5b6eceb235b9bfbeccf51b6304d0b") },
                        StorageDiff { key: feltor("0x5bdaf1d47b176bfcd1114809af85a46b9c4376e87e361d86536f0288a284b65"), value: feltor("0x28dff6722aa73281b2cf84cac09950b71fa90512db294d2042119abdd9f4b87") },
                        StorageDiff { key: feltor("0x5bdaf1d47b176bfcd1114809af85a46b9c4376e87e361d86536f0288a284b66"), value: feltor("0x57a8f8a019ccab5bfc6ff86c96b1392257abb8d5d110c01d326b94247af161c") },
                        StorageDiff { key: feltor("0x5f750dc13ed239fa6fc43ff6e10ae9125a33bd05ec034fc3bb4dd168df3505f"), value: feltor("0x7e5") },
                    ]),
                    (feltor("0x735596016a37ee972c42adef6a3cf628c19bb3794369c65d2c82ba034aecf2c"), vec![
                        StorageDiff { key: feltor("0x5"), value: feltor("0x64") },
                        StorageDiff { key: feltor("0x2f50710449a06a9fa789b3c029a63bd0b1f722f46505828a9f815cf91b31d8"), value: feltor("0x2a222e62eabe91abdb6838fa8b267ffe81a6eb575f61e96ec9aa4460c0925a2") },
                    ]),
                ].into_iter().collect(),
                nonces: HashMap::new(),
                deployed_contracts: vec![
                    DeployedContract {
                        address: feltor("0x20cfa74ee3564b4cd5435cdace0f9c4d43b939620e4a0bb5076105df0a626c6"),
                        class_hash: feltor("0x10455c752b86932ce552f2b0fe81a880746649b9aee7e0d842bf3f52378f9f8"),
                    },
                    DeployedContract {
                        address: feltor("0x31c887d82502ceb218c06ebb46198da3f7b92864a8223746bc836dda3e34b52"),
                        class_hash: feltor("0x10455c752b86932ce552f2b0fe81a880746649b9aee7e0d842bf3f52378f9f8"),
                    },
                    DeployedContract {
                        address: feltor("0x31c9cdb9b00cb35cf31c05855c0ec3ecf6f7952a1ce6e3c53c3455fcd75a280"),
                        class_hash: feltor("0x10455c752b86932ce552f2b0fe81a880746649b9aee7e0d842bf3f52378f9f8"),
                    },
                    DeployedContract {
                        address: feltor("0x6ee3440b08a9c805305449ec7f7003f27e9f7e287b83610952ec36bdc5a6bae"),
                        class_hash: feltor("0x10455c752b86932ce552f2b0fe81a880746649b9aee7e0d842bf3f52378f9f8"),
                    },
                    DeployedContract {
                        address: feltor("0x735596016a37ee972c42adef6a3cf628c19bb3794369c65d2c82ba034aecf2c"),
                        class_hash: feltor("0x10455c752b86932ce552f2b0fe81a880746649b9aee7e0d842bf3f52378f9f8"),
                    },
                ],
                declared_classes: vec![],
                old_declared_contracts: vec![],
                replaced_classes: vec![],
            },
        }
    }

    pub fn build_raw_csd() -> CommitmentStateDiff {
        CommitmentStateDiff {
            address_to_class_hash: {
                let mut map = IndexMap::new();
                // Populating with deployed contracts' addresses and their class hashes
                map.insert(feltor("0x20cfa74ee3564b4cd5435cdace0f9c4d43b939620e4a0bb5076105df0a626c6"), feltor("0x10455c752b86932ce552f2b0fe81a880746649b9aee7e0d842bf3f52378f9f8"));
                map.insert(feltor("0x31c887d82502ceb218c06ebb46198da3f7b92864a8223746bc836dda3e34b52"), feltor("0x10455c752b86932ce552f2b0fe81a880746649b9aee7e0d842bf3f52378f9f8"));
                // Add the rest of the deployed contracts here
                map
            },
            address_to_nonce: IndexMap::new(),
            storage_updates: {
                let mut map = HashMap::new();
                let mut storage_diffs = HashMap::new();
                storage_diffs.insert(feltor("0x5"), feltor("0x22b"));
                storage_diffs.insert(feltor("0x313ad57fdf765addc71329abf8d74ac2bce6d46da8c2b9b82255a5076620300"), feltor("0x4e7e989d58a17cd279eca440c5eaa829efb6f9967aaad89022acbe644c39b36"));
                // Add the rest of the storage diffs here
                map
            },
            class_hash_to_compiled_class_hash: HashMap::new(),
        }
    }

    pub async fn fetch_genesis_state_update() -> Result<StateUpdate, String> {
        let provider = SequencerGatewayProvider::new(
            Url::from_str("https://alpha-mainnet.starknet.io/gateway").unwrap(),
            Url::from_str("https://alpha-mainnet.starknet.io/feeder_gateway").unwrap(),
            starknet_core::types::FieldElement::from_byte_slice_be(b"SN_MAIN").unwrap(),
        );

        let state_update = provider
            .get_state_update(BlockId::Number(0).into())
            .await
            .map_err(|e| format!("failed to get state update: {e}"))?;

        Ok(state_update)
    }

    #[tokio::test]
    pub async fn test_build_csd() {
        let fetch_csd = fetch_genesis_state_update().await.expect("Failed to fetch genesis state update");
        let fetch_raw_csd = fetch_raw_genesis_state_update();
        let build_raw_csd = build_commitment_state_diff(StateUpdateWrapper::from(fetch_raw_csd));
        let build_fetch_csd = build_commitment_state_diff(StateUpdateWrapper::from(fetch_csd));
        assert_eq!(build_raw_csd, build_fetch_csd);
    }
}
