use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;

use futures::channel::mpsc;
use futures::{Stream, StreamExt};
use indexmap::{IndexMap, IndexSet};
use mp_hashers::HasherT;
use mp_storage::{SN_COMPILED_CLASS_HASH_PREFIX, SN_CONTRACT_CLASS_HASH_PREFIX, SN_NONCE_PREFIX, SN_STORAGE_PREFIX};
use pallet_starknet_runtime_api::StarknetRuntimeApi;
use sc_client_api::client::BlockchainEvents;
use sc_client_api::{StorageEventStream, StorageNotification};
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use sp_runtime::traits::{Block as BlockT, Header};
use starknet_api::api_core::{ClassHash, CompiledClassHash, ContractAddress, Nonce, PatriciaKey};
use starknet_api::block::BlockHash;
use starknet_api::hash::StarkFelt;
use starknet_api::state::{StorageKey as StarknetStorageKey, ThinStateDiff};
use thiserror::Error;

pub struct BlockDAData(pub BlockHash, pub ThinStateDiff, pub usize);

pub struct CommitmentStateDiffWorker<B: BlockT, C, H> {
    client: Arc<C>,
    storage_event_stream: StorageEventStream<B::Hash>,
    tx: mpsc::Sender<BlockDAData>,
    msg: Option<BlockDAData>,
    phantom: PhantomData<H>,
}

impl<B: BlockT, C, H> CommitmentStateDiffWorker<B, C, H>
where
    C: BlockchainEvents<B>,
{
    pub fn new(client: Arc<C>, tx: mpsc::Sender<BlockDAData>) -> Self {
        let storage_event_stream = client
            .storage_changes_notification_stream(None, None)
            .expect("the node storage changes notification stream should be up and running");
        Self { client, storage_event_stream, tx, msg: Default::default(), phantom: PhantomData }
    }
}

impl<B: BlockT, C, H> Stream for CommitmentStateDiffWorker<B, C, H>
where
    C: ProvideRuntimeApi<B>,
    C::Api: StarknetRuntimeApi<B>,
    C: HeaderBackend<B>,
    H: HasherT + Unpin,
{
    type Item = ();

    // CommitmentStateDiffWorker is a state machine with two states
    // state 1: waiting for some StorageEvent to happen, `commitment_state_diff` field is `None`
    // state 2: waiting for the channel to be ready, `commitment_state_diff` field is `Some`
    fn poll_next(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Option<Self::Item>> {
        let self_as_mut = self.get_mut();
        if self_as_mut.msg.is_none() {
            // State 1
            match Stream::poll_next(Pin::new(&mut self_as_mut.storage_event_stream), cx) {
                // No new block have been produced, we wait
                Poll::Pending => return Poll::Pending,

                // A new block have been produced, we process it and update our state machine
                Poll::Ready(Some(storage_notification)) => {
                    let block_hash = storage_notification.block;

                    match build_commitment_state_diff::<B, C, H>(self_as_mut.client.clone(), storage_notification) {
                        Ok(msg) => self_as_mut.msg = Some(msg),
                        Err(e) => {
                            log::error!(
                                "Block with substrate hash `{block_hash}` skiped. Failed to compute commitment state \
                                 diff: {e}",
                            );

                            return Poll::Pending;
                        }
                    }
                }

                // The stream has been close, we close too.
                // This should not happen tho
                Poll::Ready(None) => return Poll::Ready(None),
            }
        }

        // At this point self_as_mut.commitment_state_diff.is_some() == true
        // State 2
        match self_as_mut.tx.poll_ready(cx) {
            // Channel is ready, we send
            Poll::Ready(Ok(())) => {
                // Safe to unwrap cause we already handle the `None` branch
                let msg = self_as_mut.msg.take().unwrap();
                // Safe to unwrap because channel is ready
                self_as_mut.tx.start_send(msg).unwrap();

                Poll::Ready(Some(()))
            }

            // Channel is full, we wait
            Poll::Pending => Poll::Pending,

            // Channel receiver has been dropped, we close.
            // This should not happen tho
            Poll::Ready(Err(e)) => {
                log::error!("CommitmentStateDiff channel receiver has been dropped: {e}");
                Poll::Ready(None)
            }
        }
    }
}

#[derive(Debug, Error)]
pub enum BuildCommitmentStateDiffError {
    #[error("failed to interact with substrate header backend")]
    SubstrateHeaderBackend(#[from] sp_blockchain::Error),
    #[error("block not found")]
    BlockNotFound,
    #[error("digest log not found")]
    DigestLogNotFound(#[from] mp_digest_log::FindLogError),
    #[error("conversion error")]
    ConversionError,
}

fn build_commitment_state_diff<B: BlockT, C, H>(
    client: Arc<C>,
    storage_notification: StorageNotification<B::Hash>,
) -> Result<BlockDAData, BuildCommitmentStateDiffError>
where
    C: ProvideRuntimeApi<B>,
    C::Api: StarknetRuntimeApi<B>,
    C: HeaderBackend<B>,
    H: HasherT,
{
    let starknet_block_hash = {
        let header = client.header(storage_notification.block)?.ok_or(BuildCommitmentStateDiffError::BlockNotFound)?;
        let digest = header.digest();
        let block = mp_digest_log::find_starknet_block(digest)?;
        block.header().hash::<H>().into()
    };

    let mut accessed_addrs: IndexSet<ContractAddress> = IndexSet::new();
    let mut commitment_state_diff = ThinStateDiff {
        declared_classes: IndexMap::new(),
        storage_diffs: IndexMap::new(),
        nonces: IndexMap::new(),
        deployed_contracts: IndexMap::new(),
        deprecated_declared_classes: Vec::new(),
        replaced_classes: IndexMap::new(),
    };

    for (_prefix, full_storage_key, change) in storage_notification.changes.iter() {
        // The storages we are interested in all have prefix of length 32 bytes.
        // The pallet identifier takes 16 bytes, the storage one 16 bytes.
        // So if a storage key is smaller than 32 bytes,
        // the program will panic when we index it to get it's prefix
        if full_storage_key.0.len() < 32 {
            continue;
        }
        let prefix = &full_storage_key.0[..32];

        // All the `try_into` are safe to `unwrap` because we know what the storage contains
        // and therefore what size it is
        if prefix == *SN_NONCE_PREFIX {
            let contract_address =
                ContractAddress(PatriciaKey(StarkFelt(full_storage_key.0[32..].try_into().unwrap())));
            // `change` is safe to unwrap as `Nonces` storage is `ValueQuery`
            let nonce = Nonce(StarkFelt(change.unwrap().0.clone().try_into().unwrap()));
            commitment_state_diff.nonces.insert(contract_address, nonce);
            accessed_addrs.insert(contract_address);
        } else if prefix == *SN_STORAGE_PREFIX {
            let contract_address =
                ContractAddress(PatriciaKey(StarkFelt(full_storage_key.0[32..64].try_into().unwrap())));
            let storage_key = StarknetStorageKey(PatriciaKey(StarkFelt(full_storage_key.0[64..].try_into().unwrap())));
            // `change` is safe to unwrap as `StorageView` storage is `ValueQuery`
            let value = StarkFelt(change.unwrap().0.clone().try_into().unwrap());

            match commitment_state_diff.storage_diffs.get_mut(&contract_address) {
                Some(contract_storage) => {
                    contract_storage.insert(storage_key, value);
                }
                None => {
                    let mut contract_storage: IndexMap<_, _, _> = Default::default();
                    contract_storage.insert(storage_key, value);

                    commitment_state_diff.storage_diffs.insert(contract_address, contract_storage);
                }
            }
            accessed_addrs.insert(contract_address);
        } else if prefix == *SN_CONTRACT_CLASS_HASH_PREFIX {
            let contract_address =
                ContractAddress(PatriciaKey(StarkFelt(full_storage_key.0[32..].try_into().unwrap())));
            // `change` is safe to unwrap as `ContractClassHashes` storage is `ValueQuery`
            let class_hash = ClassHash(StarkFelt(change.unwrap().0.clone().try_into().unwrap()));

            // check if contract already exists
            let runtime_api = client.runtime_api();
            let current_block_hash = client.info().best_hash;

            let contract_exists = runtime_api.contract_class_by_class_hash(current_block_hash, class_hash).is_ok();

            if contract_exists {
                commitment_state_diff.replaced_classes.insert(contract_address, class_hash);
            } else {
                commitment_state_diff.deployed_contracts.insert(contract_address, class_hash);
            }
            accessed_addrs.insert(contract_address);
        } else if prefix == *SN_COMPILED_CLASS_HASH_PREFIX {
            let class_hash = ClassHash(StarkFelt(full_storage_key.0[32..].try_into().unwrap()));
            // In the current state of starknet protocol, a compiled class hash can not be erased, so we should
            // never see `change` being `None`. But there have been an "erase contract class" mechanism live on
            // the network during the Regenesis migration. Better safe than sorry.
            let compiled_class_hash =
                CompiledClassHash(change.map(|data| StarkFelt(data.0.clone().try_into().unwrap())).unwrap_or_default());

            commitment_state_diff.declared_classes.insert(class_hash, compiled_class_hash);
        }
    }

    Ok(BlockDAData(starknet_block_hash, commitment_state_diff, accessed_addrs.len()))
}

pub async fn verify_l2(mut rx: mpsc::Receiver<BlockDAData>) {
    while let Some(_rx) = rx.next().await {
        // TODO: retrieve and deal with state commitment accross L1
        // println!("➡️ block_hash {:?} with {:?}", block_hash, state_commitment(csd).0);
        // update_l2({block_number, block_hash, state_commitment})
    }
}

// /// Get L2 state commitment of a Block from a CommitmentStateDiff
// pub fn state_commitment(csd: CommitmentStateDiff) -> Felt252Wrapper {
//     let contracts_tree_root = {
//         let mut contracts_tree = StateCommitmentTree::<PedersenHasher>::default();

//         for (address, class_hash) in csd.address_to_class_hash {
//             let default_nonce = Nonce::default();
//             let nonce = csd.address_to_nonce.get(&address).unwrap_or(&default_nonce);

//             let mut storage_tree = StateCommitmentTree::<PedersenHasher>::default();

//             let default_storage_updates = IndexMap::<StarknetStorageKey, StarkFelt>::default();
//             let storage_updates =
// csd.storage_updates.get(&address).unwrap_or(&default_storage_updates);             for (key,
// value) in storage_updates {                 storage_tree.set((*key).into(), (*value).into());
//             }

//             let storage_root = storage_tree.commit();
//             let contract_state_hash = calculate_contract_state_hash::<PedersenHasher>(
//                 class_hash.into(),
//                 storage_root.into(),
//                 (*nonce).into(),
//             );

//             contracts_tree.set(address.into(), contract_state_hash);
//         }

//         contracts_tree.commit()
//     };

//     let classes_tree_root = {
//         let class_hashes: Vec<Felt252Wrapper> = csd
//             .class_hash_to_compiled_class_hash
//             .iter()
//             .map(|(_, compiled_class_hash)| {
//                 
// calculate_class_commitment_leaf_hash::<PoseidonHasher>((*compiled_class_hash).into())            
// })             .collect();
//         calculate_class_commitment_tree_root_hash::<PoseidonHasher>(&class_hashes)
//     };

//     if classes_tree_root == Felt252Wrapper::ZERO {
//         contracts_tree_root
//     } else {
//         calculate_state_commitment::<PoseidonHasher>(contracts_tree_root, classes_tree_root)
//     }
// }
