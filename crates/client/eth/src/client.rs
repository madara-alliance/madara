use crate::config::{L1StateUpdate, LogStateUpdate, StarknetContracts};
use crate::utils::{convert_log_state_update, trim_hash, u256_to_starkfelt, LOG_STATE_UPDTATE_TOPIC};
use alloy::consensus::TypedTransaction;
use alloy::hex;
use alloy::primitives::{Address, BlockNumber, B256, U64};
use alloy::providers::{Provider, ProviderBuilder, ReqwestProvider, RootProvider};
use alloy::rpc::types::{Filter, FilterSet, TransactionRequest};
use alloy::sol;
use alloy::sol_types::SolEvent;
use anyhow::{bail, Context};
use bitvec::macros::internal::funty::Fundamental;
use dc_db::DeoxysBackend;
use std::sync::Arc;
use alloy::transports::http::{Client, Http};
use dc_sync::metrics::block_metrics::BlockMetrics;
use dp_convert::{ToFelt, ToStarkFelt};
use dp_transactions::TEST_CHAIN_ID;
use dp_utils::channel_wait_or_graceful_shutdown;
use futures::StreamExt;
use primitive_types::H256;
use serde_json::Value;
use starknet_api::hash::StarkFelt;
use starknet_types_core::felt::Felt;
use url::Url;
use crate::client::StarknetCore::StarknetCoreInstance;

sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    StarknetCore,
    "src/abis/starknet_core.json"
);

pub struct EthereumClient {
    provider: Arc<ReqwestProvider>,
    l1_core_contract:  StarknetCoreInstance<Http<Client>, RootProvider<Http<Client>>>,
}

impl EthereumClient {
    /// Create a new EthereumClient instance with the given RPC URL
    pub async fn new(url: Url, l1_core_address: Address) -> anyhow::Result<Self> {
        let provider = ProviderBuilder::new().on_http(url);
        let core_contract = StarknetCore::new(l1_core_address, provider.clone());

        Ok(Self { provider: Arc::new(provider), l1_core_contract: core_contract })
    }

    /// Retrieves the latest Ethereum block number
    pub async fn get_latest_block_number(&self) -> anyhow::Result<u64> {
        let block_number = self.provider.get_block_number().await?.as_u64();
        Ok(block_number)
    }

    /// Get the block number of the last occurrence of a given event.
    pub async fn get_last_event_block_number(&self) -> anyhow::Result<u64> {
        let topic = B256::from_slice(&hex::decode(&LOG_STATE_UPDTATE_TOPIC[2..])?);
        let latest_block: u64 =
            self.get_latest_block_number().await.expect("Failed to retrieve latest block number").into();

        // Assuming an avg Block time of 15sec we check for a LogStateUpdate occurence in the last ~24h
        let filter = Filter::new()
            .from_block(latest_block - 6000)
            .to_block(latest_block)
            .address(*self.l1_core_contract.address())
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
    pub async fn get_last_verified_block_number(&self) -> anyhow::Result<u64> {
        let block_number = self.l1_core_contract.stateBlockNumber().call().await.unwrap();
        let last_block_number: u64 = (block_number._0).as_u64();
        Ok(last_block_number)
    }

    /// Get the last Starknet state root verified on L1
    pub async fn get_last_state_root(&self) -> anyhow::Result<StarkFelt> {
        let state_root = self.l1_core_contract.stateRoot().call().await.unwrap();
        u256_to_starkfelt(state_root._0)
    }

    /// Get the last Starknet block hash verified on L1
    pub async fn get_last_verified_block_hash(&self) -> anyhow::Result<StarkFelt> {
        let block_hash = self.l1_core_contract.stateBlockHash().call().await.unwrap();
        u256_to_starkfelt(block_hash._0)
    }

    /// Get the last Starknet state update verified on the L1
    pub async fn get_initial_state(client: &EthereumClient) -> anyhow::Result<L1StateUpdate> {
        let block_number = client.get_last_verified_block_number().await?;
        let block_hash = client.get_last_verified_block_hash().await?;
        let global_root = client.get_last_state_root().await?;

        Ok(L1StateUpdate { global_root, block_number, block_hash })
    }

    /// Subscribes to the LogStateUpdate event from the Starknet core contract and store latest
    /// verified state
    pub async fn listen_and_update_state(
        &self,
        backend: &DeoxysBackend,
        block_metrics: BlockMetrics,
        chain_id: Felt,
    ) -> anyhow::Result<()> {
        let event_filter = self.l1_core_contract.event_filter::<StarknetCore::LogStateUpdate>();

        let mut event_stream = event_filter.watch().await.unwrap().into_stream();

        while let Some(event_result) = channel_wait_or_graceful_shutdown(event_stream.next()).await {
            let log = event_result.context("listening for events")?;
            let format_event =
                convert_log_state_update(log.0.clone()).context("formatting event into an L1StateUpdate")?;
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
