use std::marker::PhantomData;
use std::sync::Arc;

use blockifier::execution::contract_class::ContractClass;
    PALLET_STARKNET, STARKNET_CONTRACT_ABI, STARKNET_CONTRACT_CLASS, STARKNET_CONTRACT_CLASS_HASH, STARKNET_NONCE,
};
use mc_db::storage_handler;
use parity_scale_codec::{Decode, Encode};
// Substrate
use sc_client_api::backend::{Backend, StorageProvider};
use sp_blockchain::HeaderBackend;
use sp_runtime::traits::Block as BlockT;
use sp_storage::StorageKey;
use starknet_api::api_core::{ClassHash, ContractAddress, Nonce};

use super::{storage_key_build, storage_prefix_build, StorageOverride};

/// An override for runtimes that use Schema V1
pub struct SchemaV1Override<B: BlockT, C, BE> {
    client: Arc<C>,
    _marker: PhantomData<(B, BE)>,
}

impl<B: BlockT, C, BE> SchemaV1Override<B, C, BE> {
    pub fn new(client: Arc<C>) -> Self {
        Self { client, _marker: PhantomData }
    }
}

impl<B, C, BE> SchemaV1Override<B, C, BE>
where
    B: BlockT,
    C: HeaderBackend<B> + StorageProvider<B, BE> + 'static,
    BE: Backend<B> + 'static,
{
    fn query_storage<T: Decode>(&self, block_hash: B::Hash, key: &StorageKey) -> Option<T> {
        if let Ok(Some(data)) = self.client.storage(block_hash, key) {
            if let Ok(result) = Decode::decode(&mut &data.0[..]) {
                return Some(result);
            }
        }
        None
    }
    fn encode_storage_key<T: Encode>(&self, key: &T) -> Vec<u8> {
        Encode::encode(key)
    }
}

impl<B, C, BE> StorageOverride<B> for SchemaV1Override<B, C, BE>
where
    B: BlockT,
    C: HeaderBackend<B> + StorageProvider<B, BE> + 'static,
    BE: Backend<B> + 'static,
{
    fn nonce(&self, block_hash: <B as BlockT>::Hash, address: ContractAddress) -> Option<Nonce> {
		let Ok(handler_block_number) = storage_handler::block_number() else {
			return None;
		let Ok(Some(block_number)) = handler_block_number.get(block_hash)

        self.contract_class_hash_by_address(block_hash, address)?;

        let storage_nonce_prefix = storage_prefix_build(PALLET_STARKNET, STARKNET_NONCE);
        let nonce = self.query_storage::<Nonce>(
            block_hash,
            &StorageKey(storage_key_build(storage_nonce_prefix, &self.encode_storage_key(&address))),
        );

        match nonce {
            Some(nonce) => Some(nonce),
            None => Some(Nonce::default()),
        }
    }
}
