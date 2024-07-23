use std::sync::Arc;
use alloy::consensus::TypedTransaction;
use alloy::hex::decode;
use alloy::providers::{Provider, ProviderBuilder, ReqwestProvider};
use alloy::primitives::{Address, B256, BlockNumber, U64};
use alloy::rpc::types::{Filter, FilterSet, TransactionRequest};
use alloy::sol_types::sol;
use anyhow::{bail, Context};

use primitive_types::H256;
use serde_json::Value;
use starknet_api::hash::StarkFelt;
use starknet_types_core::felt::Felt;
use url::Url;
use dc_db::DeoxysBackend;
use dp_convert::{ToFelt, ToStarkFelt};
use dp_utils::channel_wait_or_graceful_shutdown;
use crate::config::{L1StateUpdate, LogStateUpdate};
use crate::utils::LOG_STATE_UPDTATE_TOPIC;
use dc_sync::metrics::block_metrics::BlockMetrics;
use dc_sync::utility::{convert_log_state_update, trim_hash};
use dp_transactions::TEST_CHAIN_ID;

pub struct EthereumClient {
    provider: Arc<ReqwestProvider>,
    l1_core_address: Address,
}

impl EthereumClient {
    /// Create a new EthereumClient instance with the given RPC URL
    pub async fn new(url: Url, l1_core_address: Address) -> anyhow::Result<Self> {
        let provider = ProviderBuilder::new().on_http(url);

        Ok(Self { provider: Arc::new(provider), l1_core_address })
    }

    /// Get current RPC URL
    // pub fn get_url(&self) -> String {
    //     self.url.as_str().to_string()
    // }

    /// Call the Ethereum RPC endpoint with the given JSON-RPC payload
    // pub async fn call_ethereum(&self, method: &str, params: Vec<Value>) -> anyhow::Result<Value, Box<dyn std::error::Error>> {
    //     let response: Value = self.provider.request(method, params).await?;
    //     Ok(response)
    // }

    /// Retrieves the latest Ethereum block number
    pub async fn get_latest_block_number(&self) -> anyhow::Result<U64> {
        let block_number: BlockNumber = self.provider.get_block_number().await?;
        Ok(block_number.into())
    }

    /// Get the block number of the last occurrence of a given event.
    pub async fn get_last_event_block_number(&self) -> anyhow::Result<u64> {
        let topic = B256::from_slice(&hex::decode(&LOG_STATE_UPDTATE_TOPIC[2..])?);
        let latest_block: u64 = self.get_latest_block_number().await.expect("Failed to retrieve latest block number").into();

        // Assuming an avg Block time of 15sec we check for a LogStateUpdate occurence in the last ~24h
        let filter = Filter::new()
            .from_block(latest_block - 6000)
            .to_block(BlockNumber::Latest)
            .address(self.l1_core_address)
            .event_signature(topic);

        let logs = self.provider.get_logs(&filter).await?;

        if let Some(last_log) = logs.last() {
            let last_block: u64 = last_log.block_number.context("no block number in log")?;
            Ok(last_block)
        } else {
            bail!("no event found")
        }
    }

    /// Get the last Starknet block number verified on L1
    pub async fn get_last_block_number(&self) -> anyhow::Result<u64> {
        let data = decode("35befa5d")?;
        let to: Address = self.l1_core_address;
        let tx_request = TransactionRequest::new().to(to).data(data);
        let tx = TypedTransaction::Legacy(tx_request);
        let result = self.provider.call(&tx).await.expect("Failed to get last block number");
        let result_str = result.to_string();
        let hex_str = result_str.trim_start_matches("Bytes(0x").trim_end_matches(')').trim_start_matches("0x");

        let block_number = u64::from_str_radix(hex_str, 16).expect("Failed to parse block number");
        Ok(block_number)
    }

    /// Get the last Starknet state root verified on L1
    pub async fn get_last_state_root(&self) -> anyhow::Result<StarkFelt> {
        let data = decode("9588eca2")?;
        let to: Address = self.l1_core_address;
        let tx_request = TransactionRequest::new().to(to).data(data);
        let tx = TypedTransaction::Legacy(tx_request);
        let result = self.provider.call(&tx).await.expect("Failed to get last state root");
        Ok(Felt::from_hex_unchecked(&result.to_string()).to_stark_felt())
    }

    /// Get the last Starknet block hash verified on L1
    pub async fn get_last_block_hash(&self) -> anyhow::Result<StarkFelt> {
        let data = decode("0x382d83e3")?;
        let to: Address = self.l1_core_address;
        let tx_request = TransactionRequest::new().to(to).data(data);
        let tx = TypedTransaction::Legacy(tx_request);
        let result = self.provider.call(&tx).await.expect("Failed to get last block hash");
        Ok(Felt::from_hex_unchecked(&result.to_string()).to_stark_felt())
    }

    /// Get the last Starknet state update verified on the L1
    pub async fn get_initial_state(client: &EthereumClient) -> anyhow::Result<L1StateUpdate> {
        let block_number = client.get_last_block_number().await?;
        let block_hash = client.get_last_block_hash().await?;
        let global_root = client.get_last_state_root().await?;

        Ok(L1StateUpdate { global_root, block_number, block_hash })
    }

    /// Subscribes to the LogStateUpdate event from the Starknet core contract and store latest
    /// verified state
    pub async fn listen_and_update_state(
        &self,
        backend: &DeoxysBackend,
        start_block: u64,
        block_metrics: BlockMetrics,
        chain_id: Felt,
    ) -> anyhow::Result<()> {
        let client = self.provider.clone();
        let address: Address = self.l1_core_address;


        sol!(
            #[allow(missing_docs)]
            #[sol(rpc)]
            StarknetCore,
            "src/abis/starknet_core.json"
        );

        let contract = StarknetCore::new(address, client);

        let event_filter = contract.event::<LogStateUpdate>().from_block(start_block).to_block(BlockNumber::Latest);

        let mut event_stream = event_filter.stream().await.context("initiatializing event stream")?;

        while let Some(event_result) = channel_wait_or_graceful_shutdown(event_stream.next()).await {
            let log = event_result.context("listening for events")?;
            let format_event =
                convert_log_state_update(log.clone()).context("formatting event into an L1StateUpdate")?;
            update_l1(backend, format_event, block_metrics.clone(), chain_id)?;
        }

        Ok(())
    }
}

pub fn update_l1(
    backend: &DeoxysBackend,
    state_update: L1StateUpdate,
    block_metrics: BlockMetrics,
    chain_id: Felt,
) -> anyhow::Result<()> {
    // This is a provisory check to avoid updating the state with an L1StateUpdate that should not have been detected
    //
    // TODO: Remove this check when the L1StateUpdate is properly verified
    if state_update.block_number > 500000u64 || chain_id == TEST_CHAIN_ID {
        log::info!(
            "🔄 Updated L1 head #{} ({}) with state root ({})",
            state_update.block_number,
            trim_hash(&state_update.block_hash.to_felt()),
            trim_hash(&state_update.global_root.to_felt())
        );

        block_metrics.l1_block_number.set(state_update.block_number as f64);

        backend
            .write_last_confirmed_block(state_update.block_number)
            .context("Setting l1 last confirmed block number")?;
        log::debug!("update_l1: wrote last confirmed block number");
    }

    Ok(())
}
