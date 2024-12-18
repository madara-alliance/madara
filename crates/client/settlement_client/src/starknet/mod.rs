use crate::client::{ClientTrait, ClientType, CoreContractInstance};
use crate::gas_price::L1BlockMetrics;
use crate::state_update::{update_l1, StateUpdate};
use alloy::primitives::FixedBytes;
use anyhow::bail;
use async_trait::async_trait;
use bigdecimal::ToPrimitive;
use mc_db::MadaraBackend;
use mp_utils::service::ServiceContext;
use starknet_core::types::{BlockId, BlockTag, EmittedEvent, EventFilter, FunctionCall};
use starknet_core::utils::get_selector_from_name;
use starknet_providers::jsonrpc::HttpTransport;
use starknet_providers::{JsonRpcClient, Provider};
use starknet_types_core::felt::Felt;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, trace};
use url::Url;

pub struct StarknetClient {
    pub provider: Arc<JsonRpcClient<HttpTransport>>,
    pub l2_core_contract: Felt,
    pub l1_block_metrics: L1BlockMetrics,
}

#[derive(Clone)]
pub struct StarknetClientConfig {
    pub url: Url,
    pub l2_contract_address: Felt,
    pub l1_block_metrics: L1BlockMetrics,
}

impl Clone for StarknetClient {
    fn clone(&self) -> Self {
        StarknetClient {
            provider: Arc::clone(&self.provider),
            l2_core_contract: self.l2_core_contract,
            l1_block_metrics: self.l1_block_metrics.clone(),
        }
    }
}

// TODO : Remove github refs after implementing the zaun imports
// Imp ⚠️ : zaun is not yet updated with latest app chain core contract implementations
//          For this reason we are adding our own call implementations.
#[async_trait]
impl ClientTrait for StarknetClient {
    type Provider = Arc<JsonRpcClient<HttpTransport>>;
    type Config = StarknetClientConfig;

    fn get_l1_block_metrics(&self) -> &L1BlockMetrics {
        &self.l1_block_metrics
    }

    fn get_core_contract_instance(&self) -> CoreContractInstance {
        CoreContractInstance::Starknet(self.l2_core_contract)
    }

    fn get_client_type(&self) -> ClientType {
        ClientType::STARKNET
    }

    async fn new(config: Self::Config) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        let provider = JsonRpcClient::new(HttpTransport::new(config.url));
        Ok(Self {
            provider: Arc::new(provider),
            l2_core_contract: config.l2_contract_address,
            l1_block_metrics: config.l1_block_metrics,
        })
    }

    async fn get_latest_block_number(&self) -> anyhow::Result<u64> {
        let block_number = self.provider.block_number().await?;
        Ok(block_number)
    }

    async fn get_last_event_block_number(&self) -> anyhow::Result<u64> {
        let latest_block = self.get_latest_block_number().await?;
        let last_events = self
            .get_events(
                BlockId::Number(latest_block - 6000),
                BlockId::Number(latest_block),
                self.l2_core_contract,
                // taken from : https://github.com/keep-starknet-strange/piltover/blob/main/src/appchain.cairo#L102
                vec![get_selector_from_name("LogStateUpdate")?],
            )
            .await?;

        let last_update_state_event = last_events.last();
        match last_update_state_event {
            Some(event) => {
                /*
                Github Ref : https://github.com/keep-starknet-strange/piltover/blob/main/src/appchain.cairo#L101
                Event description :
                ------------------
                #[derive(Drop, starknet::Event)]
                struct LogStateUpdate {
                    state_root: felt252,
                    block_number: felt252,
                    block_hash: felt252,
                }
                */
                assert_eq!(event.data.len(), 3, "Event response invalid !!");
                Ok(event.data[1].to_u64().unwrap())
            }
            None => {
                bail!("No event found")
            }
        }
    }

    async fn get_last_verified_block_number(&self) -> anyhow::Result<u64> {
        let call_res = self
            .provider
            .call(
                FunctionCall {
                    contract_address: self.l2_core_contract,
                    /*
                    Github Ref : https://github.com/keep-starknet-strange/piltover/blob/main/src/state/component.cairo#L59
                    Function Call response : (StateRoot, BlockNumber, BlockHash)
                    */
                    entry_point_selector: get_selector_from_name("get_state")?,
                    calldata: vec![],
                },
                BlockId::Tag(BlockTag::Latest),
            )
            .await?;
        assert_eq!(call_res.len(), 3, "Call response invalid !!");
        // Block Number index in call response : 1
        Ok(call_res[1].to_u64().unwrap())
    }

    async fn get_last_state_root(&self) -> anyhow::Result<Felt> {
        let call_res = self
            .provider
            .call(
                FunctionCall {
                    contract_address: self.l2_core_contract,
                    /*
                    Github Ref : https://github.com/keep-starknet-strange/piltover/blob/main/src/state/component.cairo#L59
                    Function Call response : (StateRoot, BlockNumber, BlockHash)
                    */
                    entry_point_selector: get_selector_from_name("get_state")?,
                    calldata: vec![],
                },
                BlockId::Tag(BlockTag::Latest),
            )
            .await?;
        assert_eq!(call_res.len(), 3, "Call response invalid !!");
        // State Root index in call response : 3
        Ok(call_res[0])
    }

    async fn get_last_verified_block_hash(&self) -> anyhow::Result<Felt> {
        let call_res = self
            .provider
            .call(
                FunctionCall {
                    contract_address: self.l2_core_contract,
                    /*
                    Github Ref : https://github.com/keep-starknet-strange/piltover/blob/main/src/state/component.cairo#L59
                    Function Call response : (StateRoot, BlockNumber, BlockHash)
                    */
                    entry_point_selector: get_selector_from_name("get_state")?,
                    calldata: vec![],
                },
                BlockId::Tag(BlockTag::Latest),
            )
            .await?;
        assert_eq!(call_res.len(), 3, "Call response invalid !!");
        // Block Hash index in call response : 2
        Ok(call_res[2])
    }

    async fn get_initial_state(&self) -> anyhow::Result<StateUpdate> {
        let block_number = self.get_last_verified_block_number().await?;
        let block_hash = self.get_last_verified_block_hash().await?;
        let global_root = self.get_last_state_root().await?;

        Ok(StateUpdate { global_root, block_number, block_hash })
    }

    async fn listen_for_update_state_events(
        &self,
        backend: Arc<MadaraBackend>,
        mut ctx: ServiceContext,
    ) -> anyhow::Result<()> {
        loop {
            let events_response = ctx.run_until_cancelled(self.get_events(
                BlockId::Number(self.get_latest_block_number().await?),
                BlockId::Number(self.get_latest_block_number().await?),
                self.l2_core_contract,
                vec![get_selector_from_name("LogStateUpdate")?],
            ));

            match events_response.await {
                Some(Ok(emitted_events)) => {
                    if let Some(event) = emitted_events.last() {
                        let data = event; // Create a longer-lived binding
                        let formatted_event = StateUpdate {
                            block_number: data.data[1].to_u64().unwrap(),
                            global_root: data.data[0],
                            block_hash: data.data[2],
                        };
                        update_l1(&backend, formatted_event, self.get_l1_block_metrics())?;
                    }
                }
                Some(Err(e)) => {
                    error!("Error processing event: {:?}", e);
                }
                None => {
                    trace!("Starknet Client : No event found");
                }
            }

            sleep(Duration::from_millis(100)).await;
        }
    }

    async fn get_eth_gas_prices(&self) -> anyhow::Result<(u128, u128)> {
        Ok((0, 0))
    }

    async fn get_l1_to_l2_message_cancellations(&self, _msg_hash: FixedBytes<32>) -> anyhow::Result<Felt> {
        todo!()
    }
}

impl StarknetClient {
    async fn get_events(
        &self,
        from_block: BlockId,
        to_block: BlockId,
        contract_address: Felt,
        keys: Vec<Felt>,
    ) -> anyhow::Result<Vec<EmittedEvent>> {
        let mut event_vec = Vec::new();
        let mut page_indicator = false;
        let mut continuation_token = String::from("0");

        while !page_indicator {
            let events = self
                .provider
                .get_events(
                    EventFilter {
                        from_block: Some(from_block),
                        to_block: Some(to_block),
                        address: Some(contract_address),
                        keys: Some(vec![keys.clone()]),
                    },
                    if continuation_token == "0" { None } else { Some(continuation_token.clone()) },
                    1000,
                )
                .await?;

            event_vec.extend(events.events);
            if let Some(token) = events.continuation_token {
                continuation_token = token;
            } else {
                page_indicator = true;
            }
        }

        Ok(event_vec)
    }
}
