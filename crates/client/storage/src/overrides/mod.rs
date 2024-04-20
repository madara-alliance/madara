use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::sync::Arc;

use frame_support::{Identity, StorageHasher};
use mp_storage::StarknetStorageSchemaVersion;
use pallet_starknet_runtime_api::StarknetRuntimeApi;
use sc_client_api::{Backend, HeaderBackend, StorageProvider};
use sp_api::ProvideRuntimeApi;
use sp_io::hashing::twox_128;
use sp_runtime::traits::Block as BlockT;
use starknet_api::api_core::{ContractAddress, Nonce};

mod schema_v1_override;

pub use self::schema_v1_override::SchemaV1Override;
use crate::onchain_storage_schema;

/// A handle containing multiple entities implementing `StorageOverride`
pub struct OverrideHandle<B: BlockT> {
    /// Contains one implementation of `StorageOverride` by version of the pallet storage schema
    pub schemas: BTreeMap<StarknetStorageSchemaVersion, Box<dyn StorageOverride<B>>>,
    /// A non-failing way to retrieve the storage data
    pub fallback: Box<dyn StorageOverride<B>>,
}

#[allow(clippy::borrowed_box)]
impl<B: BlockT> OverrideHandle<B> {
    pub fn for_schema_version(&self, schema_version: &StarknetStorageSchemaVersion) -> &Box<dyn StorageOverride<B>> {
        match self.schemas.get(schema_version) {
            Some(storage_override) => storage_override,
            None => &self.fallback,
        }
    }
}

#[allow(clippy::borrowed_box)]
impl<B: BlockT> OverrideHandle<B> {
    pub fn for_block_hash<C: HeaderBackend<B> + StorageProvider<B, BE>, BE: Backend<B>>(
        &self,
        client: &C,
        block_hash: B::Hash,
    ) -> &Box<dyn StorageOverride<B>> {
        let schema_version = onchain_storage_schema(client, block_hash);
        self.for_schema_version(&schema_version)
    }
}

/// Something that can fetch Starknet-related data. This trait is quite similar to the runtime API,
/// and indeed the implementation of it uses the runtime API.
/// Having this trait is useful because it allows optimized implementations that fetch data from a
/// State Backend with some assumptions about pallet-starknet's storage schema. Using such an
/// optimized implementation avoids spawning a runtime and the overhead associated with it.
pub trait StorageOverride<B: BlockT>: Send + Sync {
    /// Returns the nonce for a provided contract address and block hash.
    fn nonce(&self, block_hash: B::Hash, address: ContractAddress) -> Option<Nonce>;
}

/// Returns the storage prefix given the pallet module name and the storage name
fn storage_prefix_build(module: &[u8], storage: &[u8]) -> Vec<u8> {
    [twox_128(module), twox_128(storage)].concat().to_vec()
}

/// Returns the storage key for single key maps using the Identity storage hasher.
fn storage_key_build(prefix: Vec<u8>, key: &[u8]) -> Vec<u8> {
    [prefix, Identity::hash(key)].concat()
}

/// A wrapper type for the Runtime API.
///
/// This type implements `StorageOverride`, so it can be used when calling the runtime API is
/// desired but a `dyn StorageOverride` is required.
pub struct RuntimeApiStorageOverride<B: BlockT, C> {
    client: Arc<C>,
    _marker: PhantomData<B>,
}

impl<B: BlockT, C> RuntimeApiStorageOverride<B, C> {
    pub fn new(client: Arc<C>) -> Self {
        Self { client, _marker: PhantomData }
    }
}

impl<B, C> StorageOverride<B> for RuntimeApiStorageOverride<B, C>
where
    B: BlockT,
    C: ProvideRuntimeApi<B> + Send + Sync,
    C::Api: StarknetRuntimeApi<B>,
{
    /// Return the nonce for a provided contract address and block hash.
    ///
    /// # Arguments
    ///
    /// * `block_hash` - The block hash
    /// * `contract_address` - The contract address to fetch the nonce for
    ///
    /// # Returns
    /// * `Some(nonce)` - The nonce for the provided contract address and block hash
    fn nonce(&self, block_hash: <B as BlockT>::Hash, contract_address: ContractAddress) -> Option<Nonce> {
        self.client.runtime_api().nonce(block_hash, contract_address).ok()
    }
}
