//! Main RPC client implementation for unified Starknet and Pathfinder access.

use anyhow::anyhow;
use futures::stream::{self, StreamExt};
use log::info;
use reqwest::Url;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider, ProviderError};
use starknet_core::types::{ConfirmedBlockId, ContractStorageKeys, StorageKey};
use starknet_types_core::felt::Felt;
use std::collections::VecDeque;
use std::sync::Arc;

use crate::constants::{MAX_CONCURRENT_PROOF_REQUESTS, MAX_STORAGE_KEYS_PER_REQUEST, STARKNET_RPC_VERSION};
use crate::types::{ClassProof, ContractProof};

pub trait ProofClient {
    fn get_proof(
        &self,
        block_number: u64,
        contract_address: Felt,
        keys: &[Felt],
    ) -> impl std::future::Future<Output = Result<ContractProof, ProviderError>> + Send;
    fn get_class_proof(
        &self,
        block_number: u64,
        class_hash: &Felt,
    ) -> impl std::future::Future<Output = Result<ClassProof, ProviderError>> + Send;
    fn get_proof_one_key(
        &self,
        block_number: u64,
        contract_address: Felt,
        key: Option<Felt>,
    ) -> impl std::future::Future<Output = Result<ContractProof, ProviderError>> + Send;
    fn get_proof_multiple_keys(
        &self,
        block_number: u64,
        contract_address: Felt,
        keys: &[Felt],
    ) -> impl std::future::Future<Output = Result<ContractProof, ProviderError>> + Send;
}

/// Internal structure containing the underlying RPC clients.
///
/// This struct encapsulates both the standard Starknet RPC client and the Pathfinder-specific
/// client, providing a unified interface for accessing different types of RPC endpoints.
struct RpcClientInner {
    /// Starknet-rs client for accessing standard Starknet RPC endpoints.
    starknet_client: JsonRpcClient<HttpTransport>,
}

impl RpcClientInner {
    /// Creates a new RPC client inner with both Starknet and Pathfinder clients.
    ///
    /// # Arguments
    ///
    /// * `base_url` - The base URL of the RPC server
    ///
    /// # Error
    ///
    /// This function will throw an error if the URL cannot be parsed or if the HTTP client
    /// cannot be created.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rpc_client::client::RpcClientInner;
    ///
    /// let inner = RpcClientInner::new("https://your-starknet-node.com");
    /// ```
    fn try_new(base_url: &str) -> anyhow::Result<Self> {
        let starknet_rpc_url = format!("{}/rpc/{}", base_url, STARKNET_RPC_VERSION);
        info!("Initializing Starknet RPC client with URL: {}", starknet_rpc_url);

        let provider = JsonRpcClient::new(HttpTransport::new(
            Url::parse(starknet_rpc_url.as_str())
                .map_err(|e| anyhow!("Failed to parse URL ({}): {}", starknet_rpc_url, e))?,
        ));

        Ok(Self { starknet_client: provider })
    }
}

/// A unified RPC client for interacting with Starknet nodes.
///
/// This client provides access to both standard Starknet RPC endpoints and Pathfinder-specific
/// extensions through a single interface. It's designed to be thread-safe and can be cloned
/// for use across multiple tasks.
///
/// # Examples
///
/// ## Basic Usage
///
/// ```rust
/// use rpc_client::RpcClient;
/// use starknet::core::types::BlockId;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let client = RpcClient::new("https://your-starknet-node.com");
///
///     // Get the latest block
///     let block = client.starknet_rpc().get_block(BlockId::Latest).await?;
///     println!("Latest block number: {}", block.block_number);
///
///     Ok(())
/// }
/// ```
#[derive(Clone)]
pub struct RpcClient {
    /// The inner client containing both Starknet and Pathfinder clients.
    inner: Arc<RpcClientInner>,
}

impl RpcClient {
    /// Creates a new RPC client with the specified base URL.
    ///
    /// # Arguments
    ///
    /// * `base_url` - The base URL of the RPC server (e.g., "https://your-starknet-node.com")
    ///
    /// # Returns
    ///
    /// A new `RpcClient` instance.
    ///
    /// # Error
    ///
    /// This function will throw an error if the URL is invalid or if the HTTP client cannot be created.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rpc_client::RpcClient;
    ///
    /// let client = RpcClient::new("https://your-starknet-node.com");
    /// ```
    pub fn try_new(base_url: &str) -> anyhow::Result<Self> {
        Ok(Self { inner: Arc::new(RpcClientInner::try_new(base_url)?) })
    }

    /// Returns a reference to the underlying Starknet RPC client.
    ///
    /// This client provides access to all standard Starknet RPC endpoints as defined
    /// in the Starknet RPC specification.
    ///
    /// # Returns
    ///
    /// A reference to the Starknet RPC client.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rpc_client::RpcClient;
    /// use starknet::core::types::BlockId;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let client = RpcClient::new("https://your-starknet-node.com");
    ///
    ///     // Use the Starknet RPC client directly
    ///     let block = client.starknet_rpc().get_block(BlockId::Latest).await?;
    ///     println!("Block hash: {:?}", block.block_hash);
    ///
    ///     Ok(())
    /// }
    /// ```
    #[must_use]
    pub fn starknet_rpc(&self) -> &JsonRpcClient<HttpTransport> {
        &self.inner.starknet_client
    }
}

impl ProofClient for JsonRpcClient<HttpTransport> {
    /// Gets storage proofs for the specified contract and keys at the given block number.
    ///
    /// This method retrieves storage proofs for multiple keys in a single contract.
    /// If no keys are provided, it will return a proof for the entire contract.
    ///
    /// # Arguments
    ///
    /// * `block_number` - The block number at which to get the proof
    /// * `contract_address` - The address of the contract
    /// * `keys` - The storage keys to get proofs for
    ///
    /// # Returns
    ///
    /// Returns a `ContractProof` containing the storage proofs or an error if the
    /// request fails.
    ///
    /// # Errors
    ///
    /// Returns a `ClientError` if the RPC request fails, the response cannot be parsed,
    /// or if there are issues with the proof conversion.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rpc_client::pathfinder::client::PathfinderRpcClient;
    /// use starknet_types_core::felt::Felt;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let client = PathfinderRpcClient::new("https://your-pathfinder-node.com");
    ///
    ///     let contract_address = Felt::from_hex("0x123...").unwrap();
    ///     let keys = vec![Felt::from_hex("0x456...").unwrap()];
    ///
    ///     let proof = client.get_proof(12345, contract_address, &keys).await?;
    ///     println!("Proof obtained for {} keys", keys.len());
    ///
    ///     Ok(())
    /// }
    /// ```
    async fn get_proof(
        &self,
        block_number: u64,
        contract_address: Felt,
        keys: &[Felt],
    ) -> Result<ContractProof, ProviderError> {
        // TODO: Return proper errors
        let mut proofs = VecDeque::new();

        if keys.is_empty() {
            // Get proof for the entire contract
            let proof = self.get_proof_one_key(block_number, contract_address, None).await?;
            proofs.push_back(proof);
        } else {
            // Get proofs for each key chunk concurrently
            let chunks: Vec<_> = keys.chunks(MAX_STORAGE_KEYS_PER_REQUEST).map(|c| c.to_vec()).collect();

            info!(
                "Fetching proofs for {} chunks with max {} concurrent requests",
                chunks.len(),
                MAX_CONCURRENT_PROOF_REQUESTS
            );

            // Create futures for all chunks and execute them concurrently
            let chunk_proofs: Vec<ContractProof> = stream::iter(chunks)
                .map(|chunk| {
                    let chunk_len = chunk.len();
                    async move {
                        info!("Calling RPC with {} keys", chunk_len);
                        self.get_proof_multiple_keys(block_number, contract_address, &chunk).await
                    }
                })
                .buffer_unordered(MAX_CONCURRENT_PROOF_REQUESTS)
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?;

            proofs.extend(chunk_proofs);
        }

        // Merge all the proofs into a single proof
        let mut proof = proofs.pop_front().ok_or(ProviderError::ArrayLengthMismatch)?;
        let contract_data = proof.contract_data.as_mut().ok_or(ProviderError::ArrayLengthMismatch)?;

        // Combine all storage proofs in contract data.
        // NOTE: storage_proof is a vector of proofs. Each of them is the union of all the paths
        // of storage keys from root to leave for a single contract.
        // So, storage_proofs.len() == num of contracts sent
        for additional_proof in proofs {
            contract_data
                .storage_proofs
                .extend(additional_proof.contract_data.ok_or(ProviderError::ArrayLengthMismatch)?.storage_proofs);
        }

        Ok(proof)
    }

    /// Gets a proof for a class at the given block number.
    ///
    /// # Arguments
    ///
    /// * `block_number` - The block number at which to get the proof
    /// * `class_hash` - The hash of the class to get the proof for
    ///
    /// # Returns
    ///
    /// Returns a `ClassProof` containing the class proof or an error if the
    /// request fails.
    ///
    /// # Errors
    ///
    /// Returns a `ClientError` if the RPC request fails or the response cannot be parsed.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rpc_client::pathfinder::client::PathfinderRpcClient;
    /// use starknet_types_core::felt::Felt;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let client = PathfinderRpcClient::new("https://your-pathfinder-node.com");
    ///
    ///     let class_hash = Felt::from_hex("0x789...").unwrap();
    ///     let proof = client.get_class_proof(12345, &class_hash).await?;
    ///     println!("Class proof obtained successfully");
    ///
    ///     Ok(())
    /// }
    /// ```
    async fn get_class_proof(&self, block_number: u64, class_hash: &Felt) -> Result<ClassProof, ProviderError> {
        info!("Querying starknet_getStorageProofs for class {:x} at block {:x}", class_hash, block_number);

        Ok(self.get_storage_proof(ConfirmedBlockId::Number(block_number), [*class_hash], [], []).await?.into())
    }

    /// Gets a proof for a single key for the given contract at the given block number.
    ///
    /// This is a helper method that gets a proof for a single storage key. For multiple
    /// keys, consider using `get_proof_multiple_keys` or `get_proof` for better efficiency.
    ///
    /// # Arguments
    ///
    /// * `block_number` - The block number at which to get the proof
    /// * `contract_address` - The address of the contract
    /// * `key` - The storage key to get the proof for, or `None` for the entire contract
    ///
    /// # Returns
    ///
    /// Returns a `ContractProof` containing the storage proof or an error if the
    /// request fails.
    ///
    /// # Errors
    ///
    /// Returns a `ClientError` if the RPC request fails or the response cannot be parsed.
    async fn get_proof_one_key(
        &self,
        block_number: u64,
        contract_address: Felt,
        key: Option<Felt>,
    ) -> Result<ContractProof, ProviderError> {
        let keys = if let Some(key) = key { vec![key] } else { Vec::new() };
        self.get_proof_multiple_keys(block_number, contract_address, &keys).await
    }

    /// Gets a proof for multiple keys for the given contract at the given block number.
    ///
    /// This method efficiently retrieves proofs for multiple storage keys in a single
    /// RPC request, up to the maximum allowed number of keys per request.
    ///
    /// # Arguments
    ///
    /// * `block_number` - The block number at which to get the proof
    /// * `contract_address` - The address of the contract
    /// * `keys` - The storage keys to get proofs for
    ///
    /// # Returns
    ///
    /// Returns a `ContractProof` containing the storage proofs or an error if the
    /// request fails.
    ///
    /// # Errors
    ///
    /// Returns a `ClientError` if the RPC request fails or the response cannot be parsed.
    async fn get_proof_multiple_keys(
        &self,
        block_number: u64,
        contract_address: Felt,
        storage_keys: &[Felt],
    ) -> Result<ContractProof, ProviderError> {
        info!(
            "Querying starknet_getStorageProof for address {:x} with {} keys at block {:x}",
            contract_address,
            storage_keys.len(),
            block_number
        );

        self.get_storage_proof(
            ConfirmedBlockId::Number(block_number),
            [],
            [contract_address],
            [ContractStorageKeys {
                contract_address,
                storage_keys: storage_keys.iter().map(|k| StorageKey(k.to_hex_string())).collect(),
            }],
        )
        .await?
        .try_into()
    }
}
