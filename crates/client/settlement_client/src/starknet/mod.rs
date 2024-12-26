use crate::client::{ClientTrait, CoreContractInstance};
use crate::gas_price::L1BlockMetrics;
use crate::messaging::MessageSent;
use crate::state_update::{update_l1, StateUpdate};
use anyhow::{anyhow, bail};
use async_trait::async_trait;
use bigdecimal::ToPrimitive;
use mc_db::l1_db::LastSyncedEventBlock;
use mc_db::MadaraBackend;
use mc_mempool::{Mempool, MempoolProvider};
use mp_utils::service::ServiceContext;
use starknet_api::core::{ChainId, ContractAddress, EntryPointSelector, Nonce};
use starknet_api::transaction::{Calldata, L1HandlerTransaction, TransactionVersion};
use starknet_core::types::{BlockId, BlockTag, EmittedEvent, EventFilter, FunctionCall};
use starknet_core::utils::get_selector_from_name;
use starknet_crypto::poseidon_hash_many;
use starknet_providers::jsonrpc::HttpTransport;
use starknet_providers::{JsonRpcClient, Provider};
use starknet_types_core::felt::Felt;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, trace};
use url::Url;

#[cfg(test)]
pub mod utils;

#[derive(Debug)]
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
    type Config = StarknetClientConfig;
    type EventStruct = MessageSent;

    fn get_l1_block_metrics(&self) -> &L1BlockMetrics {
        &self.l1_block_metrics
    }

    fn get_core_contract_instance(&self) -> CoreContractInstance {
        CoreContractInstance::Starknet(self.l2_core_contract)
    }

    async fn new(config: Self::Config) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        let provider = JsonRpcClient::new(HttpTransport::new(config.url));
        // Check if l2 contract exists :
        // If contract is not there this will error out.
        provider.get_class_at(BlockId::Tag(BlockTag::Latest), config.l2_contract_address).await?;
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
        // If block on l2 is not greater than or equal to 6000 we will consider the last block to 0.
        let last_block = if latest_block <= 6000 { 0 } else { 6000 };
        let last_events = self
            .get_events(
                BlockId::Number(last_block),
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
                GitHub Ref : https://github.com/keep-starknet-strange/piltover/blob/main/src/appchain.cairo#L101
                Event description :
                ------------------
                #[derive(Drop, starknet::Event)]
                struct LogStateUpdate {
                    state_root: felt252,
                    block_number: felt252,
                    block_hash: felt252,
                }
                */
                if event.data.len() != 3 {
                    return Err(anyhow!("Event response invalid !!"));
                }
                // Block number management in case of pending block number events.
                match event.block_number {
                    Some(block_number) => Ok(block_number),
                    None => Ok(self.get_latest_block_number().await? + 1),
                }
            }
            None => {
                bail!("No event found")
            }
        }
    }

    async fn get_last_verified_block_number(&self) -> anyhow::Result<u64> {
        // Block Number index in call response : 1
        Ok(u64::try_from(self.get_state_call().await?[1])?)
    }

    async fn get_last_state_root(&self) -> anyhow::Result<Felt> {
        // State Root index in call response : 0
        Ok(self.get_state_call().await?[0])
    }

    async fn get_last_verified_block_hash(&self) -> anyhow::Result<Felt> {
        // Block Hash index in call response : 2
        Ok(self.get_state_call().await?[2])
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
                            block_number: data.data[1].to_u64().expect("Unable to parse Felt result into u64"),
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

    async fn listen_for_messaging_events(
        &self,
        backend: Arc<MadaraBackend>,
        mut ctx: ServiceContext,
        last_synced_event_block: LastSyncedEventBlock,
        chain_id: ChainId,
        mempool: Arc<Mempool>,
    ) -> anyhow::Result<()> {
        // This is for checking if initial events are synced.
        let mut sync_flag = false;

        loop {
            let events_response = ctx.run_until_cancelled(self.get_events(
                BlockId::Number(if !sync_flag {
                    last_synced_event_block.block_number
                } else {
                    self.get_latest_block_number().await?
                }),
                BlockId::Number(self.get_latest_block_number().await?),
                self.l2_core_contract,
                vec![get_selector_from_name("MessageSent")?],
            ));

            // set synced flag as true.
            sync_flag = true;

            match events_response.await {
                Some(Ok(emitted_events)) => {
                    for event in emitted_events {
                        tracing::info!(
                            "🔵 Processing L2 Message from block: {:?}, transaction_hash: {:?}, fromAddress: {:?}",
                            event.block_number,
                            event.transaction_hash,
                            event.data[1]
                        );

                        // For payload in data :
                        // 6th element is the payload array length.
                        let mut payload_array = vec![];
                        event.data.iter().skip(6).for_each(|data| {
                            payload_array.push(*data);
                        });

                        let formatted_event = MessageSent {
                            message_hash: event.data[0],
                            from: event.data[1],
                            to: event.data[2],
                            selector: event.data[3],
                            nonce: event.data[4],
                            payload: payload_array,
                        };

                        let event_hash = self.get_messaging_hash(&formatted_event)?;
                        tracing::info!(
                            "🔵 Checking for cancelation, event hash : {:?}",
                            Felt::from_bytes_be_slice(event_hash.as_slice())
                        );
                        let cancellation_timestamp = self.get_l1_to_l2_message_cancellations(event_hash).await?;
                        if cancellation_timestamp != Felt::ZERO {
                            tracing::info!(
                                "🔵 L2 Message was cancelled in block at timestamp : {:?}",
                                cancellation_timestamp
                            );
                            let tx_nonce = Nonce(event.data[4]);
                            match backend.has_l1_messaging_nonce(tx_nonce) {
                                Ok(false) => {
                                    backend.set_l1_messaging_nonce(tx_nonce)?;
                                }
                                Ok(true) => {}
                                Err(e) => {
                                    tracing::error!("🔵 Unexpected DB error: {:?}", e);
                                    return Err(e.into());
                                }
                            };
                            continue;
                        }

                        // TODO : need to figure out what to pass instead of event index.
                        // In case of eth we are passing that and using that for indexing the events.
                        // This value is also stored in db in order to track the events.
                        // This is also crucial in case which node is killed while executing the messages.
                        match self
                            .process_message(
                                &backend,
                                &formatted_event,
                                &event.block_number,
                                &Some(0),
                                &chain_id,
                                mempool.clone(),
                            )
                            .await
                        {
                            Ok(Some(tx_hash)) => {
                                tracing::info!(
                                    "🔵 L2 Message from block: {:?}, transaction_hash: {:?} submitted, \
                                transaction hash on L2: {:?}",
                                    event.block_number,
                                    event.transaction_hash,
                                    tx_hash
                                );
                            }
                            Ok(None) => {}
                            Err(e) => {
                                tracing::error!(
                                    "🔵 Unexpected error while processing L2 Message from block: {:?}, transaction_hash: {:?}, \
                                    error: {:?}",
                                    event.block_number,
                                    event.transaction_hash,
                                    e
                                )
                            }
                        }
                    }
                }
                Some(Err(e)) => {
                    error!("Error processing event: {:?}", e);
                }
                None => {
                    trace!("Starknet Client : No event found");
                }
            }

            // TODO : take this as a block time of L2
            sleep(Duration::from_secs(5)).await;
        }
    }

    // We are returning here (0,0) because we are assuming that
    // the L3s will have zero gas prices. for any transaction.
    // So that's why we will keep the prices as 0 returning from
    // our settlement client.
    async fn get_gas_prices(&self) -> anyhow::Result<(u128, u128)> {
        Ok((0, 0))
    }

    fn get_messaging_hash(&self, event: &Self::EventStruct) -> anyhow::Result<Vec<u8>> {
        Ok(poseidon_hash_many(&self.event_to_felt_array(event)).to_bytes_be().to_vec())
    }

    async fn process_message(
        &self,
        backend: &MadaraBackend,
        event: &Self::EventStruct,
        settlement_layer_block_number: &Option<u64>,
        event_index: &Option<u64>,
        _chain_id: &ChainId,
        mempool: Arc<Mempool>,
    ) -> anyhow::Result<Option<Felt>> {
        let transaction = self.parse_handle_message_transaction(event)?;
        let tx_nonce = transaction.nonce;

        // Ensure that L2 message has not been executed
        match backend.has_l1_messaging_nonce(tx_nonce) {
            Ok(false) => {
                backend.set_l1_messaging_nonce(tx_nonce)?;
            }
            Ok(true) => {
                tracing::debug!("🔵 Event already processed: {:?}", transaction);
                return Ok(None);
            }
            Err(e) => {
                tracing::error!("🔵 Unexpected DB error: {:?}", e);
                return Err(e.into());
            }
        };

        let res = mempool.accept_l1_handler_tx(transaction.into(), 0);

        // TODO: remove unwraps
        // Ques: shall it panic if no block number of event_index?
        let block_sent = LastSyncedEventBlock::new(settlement_layer_block_number.unwrap(), event_index.unwrap());
        backend.messaging_update_last_synced_l1_block_with_event(block_sent)?;

        Ok(Some(res?.transaction_hash))
    }

    fn parse_handle_message_transaction(&self, event: &Self::EventStruct) -> anyhow::Result<L1HandlerTransaction> {
        let calldata: Calldata = {
            let mut calldata: Vec<_> = Vec::with_capacity(event.payload.len() + 1);
            calldata.push(event.from);
            calldata.extend(event.payload.clone());
            Calldata(Arc::new(calldata))
        };

        Ok(L1HandlerTransaction {
            nonce: Nonce(event.nonce),
            contract_address: ContractAddress(event.from.try_into()?),
            entry_point_selector: EntryPointSelector(event.selector),
            calldata,
            version: TransactionVersion(Felt::ZERO),
        })
    }

    async fn get_l1_to_l2_message_cancellations(&self, msg_hash: Vec<u8>) -> anyhow::Result<Felt> {
        let call_res = self
            .provider
            .call(
                FunctionCall {
                    contract_address: self.l2_core_contract,
                    // No get_message_cancellation function in pilt over as of now
                    entry_point_selector: get_selector_from_name("l1_to_l2_message_cancellations")?,
                    calldata: vec![Felt::from_bytes_be_slice(msg_hash.as_slice())],
                },
                BlockId::Tag(BlockTag::Pending),
            )
            .await?;
        // Ensure correct read call : u256 (0, 0)
        assert_eq!(call_res.len(), 2, "l1_to_l2_message_cancellations should return only 2 values");
        Ok(call_res[0])
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
        let mut continuation_token: Option<String> = None;

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
                    continuation_token.clone(),
                    1000,
                )
                .await?;

            event_vec.extend(events.events);
            if let Some(token) = events.continuation_token {
                continuation_token = Some(token);
            } else {
                page_indicator = true;
            }
        }

        Ok(event_vec)
    }

    fn event_to_felt_array(&self, event: &MessageSent) -> Vec<Felt> {
        let mut felt_vec = vec![event.from, event.to, event.selector, event.nonce];
        felt_vec.push(Felt::from(event.payload.len()));
        event.payload.clone().into_iter().for_each(|felt| {
            felt_vec.push(felt);
        });

        felt_vec
    }
}

impl StarknetClient {
    pub async fn get_state_call(&self) -> anyhow::Result<Vec<Felt>> {
        let call_res = self
            .provider
            .call(
                FunctionCall {
                    contract_address: self.l2_core_contract,
                    /*
                    GitHub Ref : https://github.com/keep-starknet-strange/piltover/blob/main/src/state/component.cairo#L59
                    Function Call response : (StateRoot, BlockNumber, BlockHash)
                    */
                    entry_point_selector: get_selector_from_name("get_state")?,
                    calldata: vec![],
                },
                BlockId::Tag(BlockTag::Pending),
            )
            .await?;
        if call_res.len() != 3 {
            return Err(anyhow!("Call response invalid !!"));
        }
        Ok(call_res)
    }
}

#[cfg(test)]
pub mod starknet_client_tests {
    use crate::client::ClientTrait;
    use crate::gas_price::L1BlockMetrics;
    use crate::starknet::utils::{prepare_starknet_client_test, send_state_update, MADARA_PORT};
    use crate::starknet::{StarknetClient, StarknetClientConfig};
    use crate::state_update::StateUpdate;
    use serial_test::serial;
    use starknet_types_core::felt::Felt;
    use std::str::FromStr;
    use std::time::Duration;
    use tokio::time::sleep;
    use url::Url;

    #[serial]
    #[tokio::test]
    async fn fail_create_new_client_contract_does_not_exists() -> anyhow::Result<()> {
        prepare_starknet_client_test().await?;
        let l1_block_metrics = L1BlockMetrics::register()?;
        let starknet_client = StarknetClient::new(StarknetClientConfig {
            url: Url::parse(format!("http://127.0.0.1:{}", MADARA_PORT).as_str())?,
            l2_contract_address: Felt::from_str("0xdeadbeef")?,
            l1_block_metrics,
        })
        .await;
        assert!(starknet_client.is_err(), "Should fail to create a new client");
        Ok(())
    }

    #[serial]
    #[tokio::test]
    async fn create_new_client_contract_exists_starknet_client() -> anyhow::Result<()> {
        let (_, deployed_address, _madara) = prepare_starknet_client_test().await?;
        let l1_block_metrics = L1BlockMetrics::register()?;
        let starknet_client = StarknetClient::new(StarknetClientConfig {
            url: Url::parse(format!("http://127.0.0.1:{}", MADARA_PORT).as_str())?,
            l2_contract_address: deployed_address,
            l1_block_metrics,
        })
        .await;
        assert!(starknet_client.is_ok(), "Should not fail to create a new client");
        Ok(())
    }

    #[serial]
    #[tokio::test]
    async fn get_last_event_block_number_works_starknet_client() -> anyhow::Result<()> {
        let (account, deployed_address, _madara) = prepare_starknet_client_test().await?;
        let l1_block_metrics = L1BlockMetrics::register()?;
        let starknet_client = StarknetClient::new(StarknetClientConfig {
            url: Url::parse(format!("http://127.0.0.1:{}", MADARA_PORT).as_str())?,
            l2_contract_address: deployed_address,
            l1_block_metrics,
        })
        .await?;

        // sending state updates :
        send_state_update(
            &account,
            deployed_address,
            StateUpdate {
                block_number: 99,
                global_root: Felt::from_hex("0xdeadbeef")?,
                block_hash: Felt::from_hex("0xdeadbeef")?,
            },
        )
        .await?;
        let last_event_block_number = send_state_update(
            &account,
            deployed_address,
            StateUpdate {
                block_number: 100,
                global_root: Felt::from_hex("0xdeadbeef")?,
                block_hash: Felt::from_hex("0xdeadbeef")?,
            },
        )
        .await?;

        // It takes time on madara for events to be stored
        sleep(Duration::from_secs(10)).await;

        let latest_event_block_number = starknet_client.get_last_event_block_number().await?;
        assert_eq!(latest_event_block_number, last_event_block_number, "Latest event should have block number 100");
        Ok(())
    }

    #[serial]
    #[tokio::test]
    async fn get_last_verified_block_hash_works_starknet_client() -> anyhow::Result<()> {
        let (account, deployed_address, _madara) = prepare_starknet_client_test().await?;
        let l1_block_metrics = L1BlockMetrics::register()?;
        let starknet_client = StarknetClient::new(StarknetClientConfig {
            url: Url::parse(format!("http://127.0.0.1:{}", MADARA_PORT).as_str())?,
            l2_contract_address: deployed_address,
            l1_block_metrics,
        })
        .await?;

        // sending state updates :
        let data_felt = Felt::from_hex("0xdeadbeef")?;
        send_state_update(
            &account,
            deployed_address,
            StateUpdate { block_number: 100, global_root: data_felt, block_hash: data_felt },
        )
        .await?;
        sleep(Duration::from_secs(5)).await;

        let last_verified_block_hash = starknet_client.get_last_verified_block_hash().await?;
        assert_eq!(last_verified_block_hash, data_felt, "Block hash should match");

        Ok(())
    }

    #[serial]
    #[tokio::test]
    async fn get_last_state_root_works_starknet_client() -> anyhow::Result<()> {
        let (account, deployed_address, _madara) = prepare_starknet_client_test().await?;
        let l1_block_metrics = L1BlockMetrics::register()?;
        let starknet_client = StarknetClient::new(StarknetClientConfig {
            url: Url::parse(format!("http://127.0.0.1:{}", MADARA_PORT).as_str())?,
            l2_contract_address: deployed_address,
            l1_block_metrics,
        })
        .await?;

        // sending state updates :
        let data_felt = Felt::from_hex("0xdeadbeef")?;
        send_state_update(
            &account,
            deployed_address,
            StateUpdate { block_number: 100, global_root: data_felt, block_hash: data_felt },
        )
        .await?;
        sleep(Duration::from_secs(5)).await;

        let last_verified_state_root = starknet_client.get_last_state_root().await?;
        assert_eq!(last_verified_state_root, data_felt, "Last state root should match");

        Ok(())
    }

    #[serial]
    #[tokio::test]
    async fn get_last_verified_block_number_works_starknet_client() -> anyhow::Result<()> {
        let (account, deployed_address, _madara) = prepare_starknet_client_test().await?;
        let l1_block_metrics = L1BlockMetrics::register()?;
        let starknet_client = StarknetClient::new(StarknetClientConfig {
            url: Url::parse(format!("http://127.0.0.1:{}", MADARA_PORT).as_str())?,
            l2_contract_address: deployed_address,
            l1_block_metrics,
        })
        .await?;

        // sending state updates :
        let data_felt = Felt::from_hex("0xdeadbeef")?;
        let block_number = 100;
        send_state_update(
            &account,
            deployed_address,
            StateUpdate { block_number, global_root: data_felt, block_hash: data_felt },
        )
        .await?;
        sleep(Duration::from_secs(5)).await;

        let last_verified_block_number = starknet_client.get_last_verified_block_number().await?;
        assert_eq!(last_verified_block_number, block_number, "Last verified block should match");

        Ok(())
    }
}
