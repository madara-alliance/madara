use blockifier::execution::contract_class::{CompiledClassV0, CompiledClassV1, RunnableCompiledClass};
use blockifier::state::errors::StateError;
use blockifier::state::state_api::{StateReader, StateResult};
use cairo_lang_starknet_classes::contract_class::version_id_from_serialized_sierra_program;
use log::{debug, warn};
use starknet::core::types::{BlockId, Felt, StarknetError};
use starknet::providers::{Provider, ProviderError};
use starknet_api::contract_class::compiled_class_hash::{HashVersion, HashableCompiledClass};
use starknet_api::contract_class::SierraVersion;
use starknet_api::core::{ClassHash, CompiledClassHash, ContractAddress, Nonce};
use starknet_api::state::StorageKey;
use starknet_os_types::deprecated_compiled_class::GenericDeprecatedCompiledClass;
use starknet_os_types::sierra_contract_class::GenericSierraContractClass;
use starknet_os_types::starknet_core_addons::decompress_starknet_legacy_contract_class;
use std::future::Future;
use std::time::Duration;
use tokio::time::sleep;

use crate::client::RpcClient;
use crate::utils::execute_coroutine;

#[cfg(test)]
pub mod tests;

/// Version of the compiled class hash algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompiledClassHashVersion {
    /// Poseidon hash (pre-SNIP-34, legacy)
    V1,
    /// BLAKE2s hash (post-SNIP-34)
    V2,
}

impl CompiledClassHashVersion {
    /// Returns the version string for logging purposes.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::V1 => "v1",
            Self::V2 => "v2",
        }
    }
}

/// Maximum number of retry attempts for RPC calls
const MAX_RETRY_ATTEMPTS: u32 = 3;

/// Initial delay for exponential backoff (in milliseconds)
const INITIAL_BACKOFF_MS: u64 = 100;

/// Maximum delay for exponential backoff (in milliseconds)
const MAX_BACKOFF_MS: u64 = 5000;

#[derive(Clone)]
pub struct AsyncRpcStateReader {
    rpc_client: RpcClient,
    block_id: Option<BlockId>,
}

impl AsyncRpcStateReader {
    pub fn new(rpc_client: RpcClient, block_id: Option<BlockId>) -> Self {
        Self { rpc_client, block_id }
    }

    /// Execute an RPC call with exponential backoff retry logic
    ///
    /// # Arguments
    /// * `operation_name` - Name of the operation for logging purposes
    /// * `f` - The async function to execute with retry logic
    ///
    /// # Returns
    /// The result of the RPC call or an error after all retries are exhausted
    async fn execute_with_retry<T, F, Fut>(&self, operation_name: &str, f: F) -> Result<T, ProviderError>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = Result<T, ProviderError>>,
    {
        let mut attempts = 0;
        let mut backoff_ms = INITIAL_BACKOFF_MS;

        loop {
            attempts += 1;

            match f().await {
                Ok(result) => {
                    if attempts > 1 {
                        debug!("{}: succeeded after {} attempts", operation_name, attempts);
                    }
                    return Ok(result);
                }
                Err(e) => {
                    // Check if the error is retryable
                    let is_retryable = match &e {
                        // Don't retry on semantic errors (contract not found, class not found, etc.)
                        ProviderError::StarknetError(StarknetError::ContractNotFound)
                        | ProviderError::StarknetError(StarknetError::ClassHashNotFound) => false,
                        // Retry on network/transport errors
                        _ => true,
                    };

                    if !is_retryable || attempts >= MAX_RETRY_ATTEMPTS {
                        if attempts > 1 {
                            warn!("{}: failed after {} attempts with error: {:?}", operation_name, attempts, e);
                        }
                        return Err(e);
                    }

                    warn!(
                        "{}: attempt {} failed with error: {:?}, retrying in {}ms...",
                        operation_name, attempts, e, backoff_ms
                    );

                    // Wait with exponential backoff
                    sleep(Duration::from_millis(backoff_ms)).await;

                    // Increase backoff for next attempt (exponential backoff with cap)
                    backoff_ms = (backoff_ms * 2).min(MAX_BACKOFF_MS);
                }
            }
        }
    }
}

// Helper function to convert provider error to state error
fn provider_error_to_state_error(provider_error: ProviderError) -> StateError {
    StateError::StateReadError(provider_error.to_string())
}

fn to_state_err<E: ToString>(e: E) -> StateError {
    StateError::StateReadError(e.to_string())
}

impl AsyncRpcStateReader {
    /// Check if there's no block_id (e.g., when processing block 0 and trying to read from block -1)
    fn has_no_block(&self) -> bool {
        self.block_id.is_none()
    }

    pub async fn get_storage_at_async(&self, contract_address: ContractAddress, key: StorageKey) -> StateResult<Felt> {
        // Return zero when no block exists (e.g., reading from block -1 when processing block 0)
        if self.has_no_block() {
            return Ok(Felt::ZERO);
        }

        let block_id = self.block_id.unwrap();
        let operation_name = format!("get_storage_at(contract: {:?}, key: {:?})", contract_address, key);

        let storage_value = match self
            .execute_with_retry(&operation_name, || {
                self.rpc_client.starknet_rpc().get_storage_at(*contract_address.key(), *key.0.key(), block_id, None)
            })
            .await
        {
            Ok(value) => Ok(value.value()),
            Err(ProviderError::StarknetError(StarknetError::ContractNotFound)) => Ok(Felt::ZERO),
            Err(e) => Err(provider_error_to_state_error(e)),
        }?;

        Ok(storage_value)
    }

    pub async fn get_nonce_at_async(&self, contract_address: ContractAddress) -> StateResult<Nonce> {
        // Return zero nonce when no block exists
        if self.has_no_block() {
            return Ok(Nonce(Felt::ZERO));
        }

        let block_id = self.block_id.unwrap();
        debug!("got a request of get_nonce_at with parameters the contract address: {:?}", contract_address);
        let operation_name = format!("get_nonce_at(contract: {:?})", contract_address);

        let nonce = match self
            .execute_with_retry(&operation_name, || {
                self.rpc_client.starknet_rpc().get_nonce(block_id, *contract_address.key())
            })
            .await
        {
            Ok(value) => Ok(value),
            Err(ProviderError::StarknetError(StarknetError::ContractNotFound)) => Ok(Felt::ZERO),
            Err(e) => Err(provider_error_to_state_error(e)),
        }?;
        Ok(Nonce(nonce))
    }

    pub async fn get_class_hash_at_async(&self, contract_address: ContractAddress) -> StateResult<ClassHash> {
        // Return default class hash when no block exists
        if self.has_no_block() {
            return Ok(ClassHash::default());
        }

        let block_id = self.block_id.unwrap();
        debug!("got a request of get_class_hash_at with parameters the contract address: {:?}", contract_address);
        let operation_name = format!("get_class_hash_at(contract: {:?})", contract_address);

        let class_hash = match self
            .execute_with_retry(&operation_name, || {
                self.rpc_client.starknet_rpc().get_class_hash_at(block_id, *contract_address.key())
            })
            .await
        {
            Ok(class_hash) => Ok(class_hash),
            Err(ProviderError::StarknetError(StarknetError::ContractNotFound)) => Ok(ClassHash::default().0),
            Err(e) => Err(provider_error_to_state_error(e)),
        }?;

        Ok(ClassHash(class_hash))
    }

    pub async fn get_compiled_class_async(&self, class_hash: ClassHash) -> StateResult<RunnableCompiledClass> {
        // When no block exists, no classes are available
        if self.has_no_block() {
            return Err(StateError::UndeclaredClassHash(class_hash));
        }

        let block_id = self.block_id.unwrap();
        debug!("got a request of get_compiled_class with parameters the class hash: {:?}", class_hash);
        let operation_name = format!("get_compiled_class(class_hash: {:?})", class_hash);

        let contract_class = match self
            .execute_with_retry(&operation_name, || self.rpc_client.starknet_rpc().get_class(block_id, class_hash.0))
            .await
        {
            Ok(contract_class) => Ok(contract_class),
            // If the ContractClass is declared in the current block,
            // might trigger this error when trying to get it on the previous block.
            // Returning an `UndeclaredClassHash` allows blockifier to continue execution
            // Reference: https://github.com/starkware-libs/sequencer/blob/1ade15c645882e3a0bd70ef8f79b23fc66a517e0/crates/blockifier/src/state/cached_state.rs#L178-L200
            Err(ProviderError::StarknetError(StarknetError::ClassHashNotFound)) => {
                Err(StateError::UndeclaredClassHash(ClassHash(class_hash.0)))
            }
            Err(e) => Err(provider_error_to_state_error(e)),
        }?;

        let runnable_contract_class: RunnableCompiledClass = match contract_class {
            starknet::core::types::ContractClass::Sierra(sierra_class) => {
                convert_sierra_to_runnable(sierra_class).map_err(to_state_err)?
            }
            starknet::core::types::ContractClass::Legacy(legacy_class) => {
                // Now use the decompression function from starknet_core_addons
                let decompressed_legacy_class =
                    decompress_starknet_legacy_contract_class(legacy_class).map_err(|e| {
                        StateError::StateReadError(format!("Failed to decompress legacy contract class: {}", e))
                    })?;

                // Convert the decompressed LegacyContractClass to GenericDeprecatedCompiledClass
                let generic_deprecated =
                    GenericDeprecatedCompiledClass::try_from(decompressed_legacy_class).map_err(to_state_err)?;
                let deprecated_contract_class =
                    generic_deprecated.to_blockifier_contract_class().map_err(to_state_err)?;

                // Convert DeprecatedContractClass to CompiledClassV0 using TryFrom
                let compiled_class_v0 = CompiledClassV0::try_from(deprecated_contract_class).map_err(|e| {
                    StateError::StateReadError(format!(
                        "Failed to convert DeprecatedContractClass to CompiledClassV0: {}",
                        e
                    ))
                })?;

                RunnableCompiledClass::V0(compiled_class_v0)
            }
        };

        Ok(runnable_contract_class)
    }

    pub async fn get_compiled_class_hash_async(&self, class_hash: ClassHash) -> StateResult<CompiledClassHash> {
        self.get_compiled_class_hash_with_version_async(class_hash, CompiledClassHashVersion::V1).await
    }

    pub async fn get_compiled_class_hash_v2_async(&self, class_hash: ClassHash) -> StateResult<CompiledClassHash> {
        self.get_compiled_class_hash_with_version_async(class_hash, CompiledClassHashVersion::V2).await
    }

    /// Internal async method for fetching compiled class hash with version selection.
    async fn get_compiled_class_hash_with_version_async(
        &self,
        class_hash: ClassHash,
        version: CompiledClassHashVersion,
    ) -> StateResult<CompiledClassHash> {
        if self.has_no_block() {
            return Err(StateError::UndeclaredClassHash(class_hash));
        }

        let block_id = self.block_id.unwrap();
        debug!("get_compiled_class_hash_{} for class_hash: {:?}", version.as_str(), class_hash);
        let operation_name = format!("get_compiled_class_hash_{}(class_hash: {:?})", version.as_str(), class_hash);

        let contract_class = self
            .execute_with_retry(&operation_name, || self.rpc_client.starknet_rpc().get_class(block_id, class_hash.0))
            .await
            .map_err(provider_error_to_state_error)?;

        compute_compiled_class_hash_internal(&contract_class, version)
    }

    pub fn get_compiled_class_hash_v2(&self, class_hash: ClassHash) -> StateResult<CompiledClassHash> {
        execute_coroutine(self.get_compiled_class_hash_v2_async(class_hash))
    }
}

// Implementing StateReader for AsyncRpcStateReader using coroutines
impl StateReader for AsyncRpcStateReader {
    fn get_storage_at(&self, contract_address: ContractAddress, key: StorageKey) -> StateResult<Felt> {
        execute_coroutine(self.get_storage_at_async(contract_address, key))
    }

    fn get_nonce_at(&self, contract_address: ContractAddress) -> StateResult<Nonce> {
        execute_coroutine(self.get_nonce_at_async(contract_address))
    }

    fn get_class_hash_at(&self, contract_address: ContractAddress) -> StateResult<ClassHash> {
        execute_coroutine(self.get_class_hash_at_async(contract_address))
    }

    fn get_compiled_class(&self, class_hash: ClassHash) -> StateResult<RunnableCompiledClass> {
        execute_coroutine(self.get_compiled_class_async(class_hash))
    }

    fn get_compiled_class_hash(&self, class_hash: ClassHash) -> StateResult<CompiledClassHash> {
        execute_coroutine(self.get_compiled_class_hash_async(class_hash))
    }

    fn get_compiled_class_hash_v2(
        &self,
        class_hash: ClassHash,
        compiled_class: &RunnableCompiledClass,
    ) -> StateResult<CompiledClassHash> {
        match compiled_class {
            RunnableCompiledClass::V0(_) => Ok(CompiledClassHash(class_hash.0)),
            RunnableCompiledClass::V1(compiled_class_v1) => Ok(compiled_class_v1.hash(&HashVersion::V2)),
            RunnableCompiledClass::V1Native(compiled_class_v1_native) => {
                Ok(compiled_class_v1_native.hash(&HashVersion::V2))
            }
        }
    }
}

/// Computes the compiled class hash for a given contract class (Poseidon, pre-SNIP-34).
/// For BLAKE2s hash (post-SNIP-34), use [`compute_compiled_class_hash_v2`].
pub fn compute_compiled_class_hash(
    contract_class: &starknet::core::types::ContractClass,
) -> Result<CompiledClassHash, StateError> {
    compute_compiled_class_hash_internal(contract_class, CompiledClassHashVersion::V1)
}

/// Computes the compiled class hash v2 (BLAKE2s) for a given contract class (SNIP-34).
pub fn compute_compiled_class_hash_v2(
    contract_class: &starknet::core::types::ContractClass,
) -> Result<CompiledClassHash, StateError> {
    compute_compiled_class_hash_internal(contract_class, CompiledClassHashVersion::V2)
}

/// Internal helper for computing compiled class hash with version selection.
fn compute_compiled_class_hash_internal(
    contract_class: &starknet::core::types::ContractClass,
    version: CompiledClassHashVersion,
) -> Result<CompiledClassHash, StateError> {
    let class_hash = match contract_class {
        starknet::core::types::ContractClass::Sierra(sierra_class) => {
            let generic_sierra = convert_sierra_class_for_generic(sierra_class)?;
            let compiled_class = generic_sierra.compile().map_err(to_state_err)?;
            match version {
                CompiledClassHashVersion::V1 => compiled_class.class_hash().map_err(to_state_err)?,
                CompiledClassHashVersion::V2 => compiled_class.class_hash_v2().map_err(to_state_err)?,
            }
        }
        starknet::core::types::ContractClass::Legacy(_) => {
            // Cairo 0 (legacy) contracts do not have a compiled class hash.
            // Return default (zero) to signal this to the blockifier's migration logic.
            // The `should_migrate` function in blockifier expects zero for Cairo 0 contracts
            // to skip them during casm hash migration.
            return Ok(CompiledClassHash::default());
        }
    };

    Ok(class_hash.into())
}

/// Convert Sierra class to a format that GenericSierraContractClass can handle.
/// This properly handles the ABI field conversion by going through starknet_core types.
fn convert_sierra_class_for_generic(
    sierra_class: &starknet::core::types::FlattenedSierraClass,
) -> Result<GenericSierraContractClass, StateError> {
    // Convert starknet::core types to starknet_core types via JSON serialization
    // This handles the type differences between the crates
    let sierra_json = serde_json::to_string(sierra_class).map_err(to_state_err)?;
    let starknet_core_sierra: starknet_core::types::FlattenedSierraClass =
        serde_json::from_str(&sierra_json).map_err(to_state_err)?;

    // Use the `From` implementation that properly handles ABI conversion
    let generic_sierra = GenericSierraContractClass::from(starknet_core_sierra);
    Ok(generic_sierra)
}

/// Convert a Sierra contract class using the improved Generic types.
/// This function uses the approach from snos-core for better type handling.
fn convert_sierra_to_runnable(
    sierra_class: starknet::core::types::FlattenedSierraClass,
) -> Result<RunnableCompiledClass, StateError> {
    debug!("Converting Sierra contract class using GenericSierraContractClass...");

    // Convert to GenericSierraContractClass using proper From implementation
    let generic_sierra = convert_sierra_class_for_generic(&sierra_class)?;

    // Get the cairo-lang contract class for version extraction
    let generic_cairo_lang_class = generic_sierra.get_cairo_lang_contract_class().map_err(to_state_err)?;

    let (version_id, _) =
        version_id_from_serialized_sierra_program(&generic_cairo_lang_class.sierra_program).map_err(to_state_err)?;

    let sierra_version = SierraVersion::new(
        version_id.major.try_into().map_err(to_state_err)?,
        version_id.minor.try_into().map_err(to_state_err)?,
        version_id.patch.try_into().map_err(to_state_err)?,
    );

    // Try compilation
    match generic_sierra.compile() {
        Ok(compiled_class) => {
            debug!("✅ Sierra compilation succeeded!");
            let versioned_casm = compiled_class.to_blockifier_contract_class(sierra_version).map_err(to_state_err)?;

            // Convert VersionedCasm to CompiledClassV1 using TryFrom
            let compiled_class_v1 = CompiledClassV1::try_from(versioned_casm).map_err(|e| {
                StateError::StateReadError(format!("Failed to convert VersionedCasm to CompiledClassV1: {}", e))
            })?;

            Ok(RunnableCompiledClass::V1(compiled_class_v1))
        }
        Err(e) => {
            warn!("⚠️  Sierra compilation failed: {}", e);
            Err(StateError::StateReadError(format!("Sierra compilation failed: {}", e)))
        }
    }
}
