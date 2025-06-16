use crate::client::{ClientType, SettlementLayerProvider};
use crate::error::SettlementClientError;
use crate::messaging::MessageToL2WithMetadata;
use crate::starknet::error::StarknetClientError;
use crate::starknet::event::{watch_events, WatchEventFilter};
use crate::state_update::{StateUpdate, StateUpdateWorker};
use async_trait::async_trait;
use bigdecimal::ToPrimitive;
use futures::stream::BoxStream;
use futures::{StreamExt, TryStreamExt};
use mp_transactions::L1HandlerTransactionWithFee;
use mp_utils::service::ServiceContext;
use starknet_core::types::{BlockId, BlockTag, EmittedEvent, EventFilter, FunctionCall, MaybePendingBlockWithTxHashes};
use starknet_core::utils::get_selector_from_name;
use starknet_crypto::poseidon_hash_many;
use starknet_providers::jsonrpc::HttpTransport;
use starknet_providers::{JsonRpcClient, Provider};
use starknet_types_core::felt::Felt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use url::Url;

pub mod error;
pub mod event;
#[cfg(test)]
pub mod test_utils;

// when the block 0 is not settled yet, this should be prev block number, this would be the output from the snos as well while
// executing the block 0.
// link: https://github.com/starkware-libs/cairo-lang/blob/master/src/starkware/starknet/solidity/StarknetState.sol#L32
const INITIAL_STATE_BLOCK_NUMBER: &str = "0x800000000000011000000000000000000000000000000000000000000000000";

#[derive(Debug)]
pub struct StarknetClient {
    pub provider: Arc<JsonRpcClient<HttpTransport>>,
    pub core_contract_address: Felt,
    pub processed_update_state_block: AtomicU64,
}

#[derive(Clone)]
pub struct StarknetClientConfig {
    pub rpc_url: Url,
    pub core_contract_address: String,
}

impl Clone for StarknetClient {
    fn clone(&self) -> Self {
        StarknetClient {
            provider: Arc::clone(&self.provider),
            core_contract_address: self.core_contract_address,
            processed_update_state_block: AtomicU64::new(self.processed_update_state_block.load(Ordering::Relaxed)),
        }
    }
}

// Add this new implementation block for constructor
impl StarknetClient {
    pub async fn new(config: StarknetClientConfig) -> Result<Self, SettlementClientError> {
        let provider = JsonRpcClient::new(HttpTransport::new(config.rpc_url));
        let core_contract_address =
            Felt::from_hex(&config.core_contract_address).map_err(|e| -> SettlementClientError {
                StarknetClientError::Conversion(format!("Invalid core contract address: {e}")).into()
            })?;
        // Check if l2 contract exists
        provider.get_class_at(BlockId::Tag(BlockTag::Latest), core_contract_address).await.map_err(
            |e| -> SettlementClientError {
                StarknetClientError::NetworkConnection { message: format!("Failed to connect to L2 contract: {}", e) }
                    .into()
            },
        )?;

        Ok(Self {
            provider: Arc::new(provider),
            core_contract_address,
            processed_update_state_block: AtomicU64::new(0), // Keeping this as 0 initially when client is initialized.
        })
    }
}

// TODO : Remove github refs after implementing the zaun imports
// Imp ⚠️ : zaun is not yet updated with latest app chain core contract implementations
//          For this reason we are adding our own call implementations.
#[async_trait]
impl SettlementLayerProvider for StarknetClient {
    fn get_client_type(&self) -> ClientType {
        ClientType::Starknet
    }

    async fn get_latest_block_number(&self) -> Result<u64, SettlementClientError> {
        self.provider.block_number().await.map_err(|e| -> SettlementClientError {
            StarknetClientError::NetworkConnection { message: format!("Failed to fetch latest block number: {}", e) }
                .into()
        })
    }

    async fn get_last_event_block_number(&self) -> Result<u64, SettlementClientError> {
        let latest_block = self.get_latest_block_number().await?;
        // If block on l2 is not greater than or equal to 6000 we will consider the last block to 0.
        let last_block = latest_block.saturating_sub(6000);
        let last_events = self
            .get_events(
                BlockId::Number(last_block),
                BlockId::Number(latest_block),
                self.core_contract_address,
                vec![get_selector_from_name("LogStateUpdate").map_err(|e| -> SettlementClientError {
                    StarknetClientError::EventSubscription {
                        message: format!("Failed to get LogStateUpdate selector: {}", e),
                    }
                    .into()
                })?],
            )
            .await?;
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
        let last_update_state_event = last_events.last().ok_or_else(|| -> SettlementClientError {
            StarknetClientError::EventProcessing {
                message: "No event found".to_string(),
                event_id: "LogStateUpdate".to_string(),
            }
            .into()
        })?;

        if last_update_state_event.data.len() != 3 {
            return Err(SettlementClientError::Starknet(StarknetClientError::InvalidResponseFormat {
                message: "LogStateUpdate event should contain exactly 3 data values".to_string(),
            }));
        }

        match last_update_state_event.block_number {
            Some(block_number) => Ok(block_number),
            None => Ok(self.get_latest_block_number().await? + 1),
        }
    }

    async fn get_current_core_contract_state(&self) -> Result<StateUpdate, SettlementClientError> {
        let state = self.get_state_call().await?; // Returns (StateRoot, BlockNumber, BlockHash).
        let global_root = state[0];
        let block_number = if state[1] == Felt::from_hex(INITIAL_STATE_BLOCK_NUMBER).unwrap() {
            None
        } else {
            u64::try_from(state[1]).ok()
        };
        let block_hash = state[2];

        Ok(StateUpdate { global_root, block_number, block_hash })
    }

    async fn listen_for_update_state_events(
        &self,
        mut ctx: ServiceContext,
        worker: StateUpdateWorker,
    ) -> Result<(), SettlementClientError> {
        while let Some(events) = ctx
            .run_until_cancelled(async {
                let latest_block = self.get_latest_block_number().await?;
                let selector = get_selector_from_name("LogStateUpdate").map_err(|e| -> SettlementClientError {
                    StarknetClientError::EventSubscription {
                        message: format!("Failed to get LogStateUpdate selector: {}", e),
                    }
                    .into()
                })?;

                self.get_events(
                    BlockId::Number(latest_block),
                    BlockId::Number(latest_block),
                    self.core_contract_address,
                    vec![selector],
                )
                .await
            })
            .await
        {
            let events_fetched = events?;
            if let Some(event) = events_fetched.last() {
                let data = event;
                let block_number = data
                    .data
                    .get(1)
                    .ok_or_else(|| -> SettlementClientError {
                        StarknetClientError::MissingField("block_number").into()
                    })?
                    .to_u64()
                    .ok_or_else(|| -> SettlementClientError {
                        StarknetClientError::Conversion("Block number conversion failed".to_string()).into()
                    })?;

                let current_processed = self.processed_update_state_block.load(Ordering::Relaxed);
                if current_processed < block_number {
                    let global_root = data.data.first().ok_or_else(|| -> SettlementClientError {
                        StarknetClientError::MissingField("global_root").into()
                    })?;
                    let block_hash = data.data.get(2).ok_or_else(|| -> SettlementClientError {
                        StarknetClientError::MissingField("block_hash").into()
                    })?;

                    let formatted_event = StateUpdate {
                        block_number: Some(block_number),
                        global_root: *global_root,
                        block_hash: *block_hash,
                    };

                    worker.update_state(formatted_event).map_err(|e| -> SettlementClientError {
                        StarknetClientError::StateSync { message: e.to_string(), block_number }.into()
                    })?;

                    self.processed_update_state_block.store(block_number, Ordering::Relaxed);
                }
            }

            sleep(Duration::from_millis(100)).await;
        }
        Ok(())
    }

    // We are returning here (0,0) because we are assuming that
    // the L3s will have zero gas prices. for any transaction.
    // So that's why we will keep the prices as 0 returning from
    // our settlement client.
    async fn get_gas_prices(&self) -> Result<(u128, u128), SettlementClientError> {
        Ok((0, 0))
    }

    fn get_messaging_hash(&self, event: &L1HandlerTransactionWithFee) -> Result<Vec<u8>, SettlementClientError> {
        Ok(poseidon_hash_many(&self.event_to_felts(event).collect::<Vec<_>>()).to_bytes_be().to_vec())
    }

    async fn message_to_l2_has_cancel_request(&self, msg_hash: &[u8]) -> Result<bool, SettlementClientError> {
        // HACK: there is no accessor the cancellation requests in piltover. That's sad. We hack something here using get events.
        // https://github.com/keep-starknet-strange/piltover/blob/a7dc4141fd21300f6d7c23b87d496004a739f430/src/messaging/component.cairo#L110C16-L110C42
        let event_selector =
            get_selector_from_name("MessageCancellationStarted").map_err(|e| -> SettlementClientError {
                StarknetClientError::L1ToL2Messaging {
                    message: format!("Failed to get MessageCancellationStarted selector: {}", e),
                }
                .into()
            })?;
        let res = self
            .provider
            .get_events(
                EventFilter {
                    address: Some(self.core_contract_address),
                    from_block: None,
                    to_block: None,
                    keys: Some(vec![vec![event_selector, Felt::from_bytes_be_slice(msg_hash)]]),
                },
                /* continuation_token */ None,
                /* batch_size */ 1,
            )
            .await
            .map_err(|e| -> SettlementClientError {
                StarknetClientError::L1ToL2Messaging {
                    message: format!("Failed to check if message has a cancellation request: {e}"),
                }
                .into()
            })?;

        Ok(!res.events.is_empty())
    }

    async fn message_to_l2_is_pending(&self, msg_hash: &[u8]) -> Result<bool, SettlementClientError> {
        // function name taken from: https://github.com/keep-starknet-strange/piltover/blob/main/src/messaging/interface.cairo#L56
        let call_res = self
            .provider
            .call(
                FunctionCall {
                    contract_address: self.core_contract_address,
                    entry_point_selector: get_selector_from_name("sn_to_appchain_messages").map_err(
                        |e| -> SettlementClientError {
                            StarknetClientError::L1ToL2Messaging {
                                message: format!("Failed to get sn_to_appchain_messages selector: {}", e),
                            }
                            .into()
                        },
                    )?,
                    calldata: vec![Felt::from_bytes_be_slice(msg_hash)],
                },
                BlockId::Tag(BlockTag::Pending),
            )
            .await
            .map_err(|e| -> SettlementClientError {
                StarknetClientError::L1ToL2Messaging {
                    message: format!("Failed to call message cancellation function: {}", e),
                }
                .into()
            })?;

        // here is the output of the call: https://github.com/keep-starknet-strange/piltover/blob/161cb3f66d256e4d1211c6b50e5d353afb713a3e/src/messaging/types.cairo#L5
        // pub enum MessageToAppchainStatus {
        //     #[default]
        //     NotSent,
        //     Sealed,
        //     Cancelled,
        //     Pending: Nonce // In Pending case, we get [3, nonce]
        // }

        tracing::debug!("Message status response length: {}, content: {:?}", call_res.len(), call_res);

        #[derive(Debug)]
        enum MessageToAppchainStatus {
            NotSent,
            Sealed,
            Cancelled,
            #[allow(dead_code)]
            Pending(/* nonce */ u64),
        }

        // Handle cases based on response length
        let status = match &call_res[..] {
            [status] if *status == Felt::ZERO => MessageToAppchainStatus::NotSent,
            [status] if *status == Felt::ONE => MessageToAppchainStatus::Sealed,
            [status] if *status == Felt::TWO => MessageToAppchainStatus::Cancelled,
            [status, nonce] if *status == Felt::THREE => {
                MessageToAppchainStatus::Pending(nonce.to_u64().ok_or_else(|| {
                    SettlementClientError::Starknet(StarknetClientError::InvalidResponseFormat {
                        message: format!("Nonce overflows u64: {nonce:#x}"),
                    })
                })?)
            }
            _ => {
                return Err(SettlementClientError::Starknet(StarknetClientError::InvalidResponseFormat {
                    message: format!("Unexpected sn_to_appchain_messages response: {:#x?}", call_res),
                }));
            }
        };

        tracing::debug!("Message status is: {:?}", status);

        Ok(matches!(status, MessageToAppchainStatus::Pending(_)))
    }

    async fn get_block_n_timestamp(&self, l1_block_n: u64) -> Result<u64, SettlementClientError> {
        tracing::debug!("get_block_n_timestamp l1_block_n={l1_block_n}");
        let block = self.provider.get_block_with_tx_hashes(BlockId::Number(l1_block_n)).await.map_err(
            |e| -> SettlementClientError {
                StarknetClientError::Provider(format!("Could not get block timestamp: {}", e)).into()
            },
        )?;

        match block {
            MaybePendingBlockWithTxHashes::Block(b) => Ok(b.timestamp),
            MaybePendingBlockWithTxHashes::PendingBlock(b) => Ok(b.timestamp),
        }
    }

    async fn messages_to_l2_stream(
        &self,
        from_l1_block_n: u64,
    ) -> Result<BoxStream<'static, Result<MessageToL2WithMetadata, SettlementClientError>>, SettlementClientError> {
        Ok(watch_events(
            self.provider.clone(),
            Some(from_l1_block_n),
            WatchEventFilter {
                address: Some(self.core_contract_address),
                keys: Some(vec![vec![get_selector_from_name("MessageSent").map_err(
                    |e| -> SettlementClientError {
                        StarknetClientError::MessageProcessing {
                            message: format!("Failed to get MessageSent selector: {}", e),
                        }
                        .into()
                    },
                )?]]),
            },
            /* polling_interval */ Duration::from_secs(1),
            /* chunk_size */ 1000,
        )
        .map_err(|e| SettlementClientError::from(StarknetClientError::Provider(format!("Provider error: {e:#}"))))
        .map(|r| r.and_then(MessageToL2WithMetadata::try_from))
        .boxed())
    }
}

impl StarknetClient {
    async fn get_events(
        &self,
        from_block: BlockId,
        to_block: BlockId,
        contract_address: Felt,
        keys: Vec<Felt>,
    ) -> Result<Vec<EmittedEvent>, SettlementClientError> {
        let mut event_vec = Vec::new();
        let mut page_indicator = false;
        let mut continuation_token: Option<String> = None;

        while !page_indicator {
            let filter = EventFilter {
                from_block: Some(from_block),
                to_block: Some(to_block),
                address: Some(contract_address),
                keys: Some(vec![keys.clone()]),
            };
            tracing::debug!("Getting events filter={filter:?} cont={continuation_token:?}");
            let events = self.provider.get_events(filter, continuation_token.clone(), 1000).await.map_err(
                |e| -> SettlementClientError {
                    StarknetClientError::EventSubscription { message: format!("Failed to fetch events: {}", e) }.into()
                },
            )?;

            event_vec.extend(events.events);
            if let Some(token) = events.continuation_token {
                continuation_token = Some(token);
            } else {
                page_indicator = true;
            }
        }

        Ok(event_vec)
    }

    fn event_to_felts<'a>(&self, event: &'a L1HandlerTransactionWithFee) -> impl Iterator<Item = Felt> + 'a {
        std::iter::once(/* from */ event.tx.calldata.first().copied().unwrap_or_default())
            .chain(std::iter::once(/* to */ event.tx.contract_address))
            .chain(std::iter::once(Felt::from(event.tx.nonce)))
            .chain(std::iter::once(event.tx.entry_point_selector))
            .chain(std::iter::once(/* payload len */ Felt::from(event.tx.calldata.len().saturating_sub(1))))
            .chain(event.tx.calldata.iter().copied().skip(1))
    }

    pub async fn get_state_call(&self) -> Result<Vec<Felt>, SettlementClientError> {
        let call_res = self
            .provider
            .call(
                FunctionCall {
                    contract_address: self.core_contract_address,
                    /*
                    GitHub Ref : https://github.com/keep-starknet-strange/piltover/blob/main/src/state/component.cairo#L59
                    Function Call response : (StateRoot, BlockNumber, BlockHash)
                    */
                    entry_point_selector: get_selector_from_name("get_state").map_err(
                        |e| -> SettlementClientError {
                            StarknetClientError::StateInitialization {
                                message: format!("Failed to get get_state selector: {}", e),
                            }
                            .into()
                        },
                    )?,
                    calldata: vec![],
                },
                BlockId::Tag(BlockTag::Pending),
            )
            .await
            .map_err(|e| -> SettlementClientError {
                StarknetClientError::StateInitialization { message: format!("Failed to get state: {}", e) }.into()
            })?;

        if call_res.len() != 3 {
            return Err(SettlementClientError::Starknet(StarknetClientError::InvalidResponseFormat {
                message: "State call response should contain exactly 3 values (state_root, block_number, block_hash)"
                    .to_string(),
            }));
        }
        Ok(call_res)
    }
}

#[cfg(test)]
pub mod starknet_client_tests {
    use super::*;
    use crate::client::SettlementLayerProvider;
    use crate::starknet::test_utils::{get_test_context, send_state_update};
    use crate::starknet::{StarknetClient, StarknetClientConfig};
    use crate::state_update::StateUpdate;
    use mp_transactions::L1HandlerTransaction;
    use rstest::*;
    use starknet_accounts::ConnectedAccount;
    use starknet_core::types::BlockId;
    use starknet_core::types::MaybePendingBlockWithTxHashes::{Block, PendingBlock};
    use starknet_providers::jsonrpc::HttpTransport;
    use starknet_providers::ProviderError::StarknetError;
    use starknet_providers::{JsonRpcClient, Provider};
    use starknet_types_core::felt::Felt;
    use std::time::Duration;
    use tokio::time::sleep;

    /// This struct holds all commonly used test resources
    pub struct StarknetClientTextFixture {
        pub context: crate::starknet::test_utils::TestContext,
        pub client: StarknetClient,
    }

    #[fixture]
    async fn test_fixture() -> StarknetClientTextFixture {
        let context = get_test_context().await;

        // Create the client
        let client = StarknetClient::new(StarknetClientConfig {
            rpc_url: context.cmd.rpc_url().parse().unwrap(),
            core_contract_address: context.deployed_appchain_contract_address.to_hex_string(),
        })
        .await
        .unwrap();

        // Return all resources bundled together
        StarknetClientTextFixture { context, client }
    }

    #[rstest]
    #[tokio::test]
    async fn fail_create_new_client_contract_does_not_exists(
        #[future] test_fixture: StarknetClientTextFixture,
    ) -> anyhow::Result<()> {
        let fixture = test_fixture.await;

        let starknet_client = StarknetClient::new(StarknetClientConfig {
            rpc_url: fixture.context.cmd.rpc_url().parse().unwrap(),
            core_contract_address: "0xdeadbeef".to_string(),
        })
        .await;
        assert!(starknet_client.is_err(), "Should fail to create a new client");

        Ok(())
    }

    #[rstest]
    #[tokio::test]
    async fn create_new_client_contract_exists_starknet_client(
        #[future] test_fixture: StarknetClientTextFixture,
    ) -> anyhow::Result<()> {
        let fixture = test_fixture.await;

        assert!(fixture.client.get_latest_block_number().await.is_ok(), "Should not fail to create a new client");

        Ok(())
    }

    // data taken from: https://sepolia.voyager.online/event/667945_6_3
    #[rstest]
    #[tokio::test]
    async fn test_get_messaging_hash(#[future] test_fixture: StarknetClientTextFixture) -> anyhow::Result<()> {
        let fixture = test_fixture.await;
        let event = L1HandlerTransactionWithFee::new(
            L1HandlerTransaction {
                version: Felt::ZERO,
                nonce: 0x2ea,
                contract_address: Felt::from_hex("0x8ff0d8c01af0b9e5ab904f0299e6ae3a94b28c680b821ab02b978447d2da67")
                    .expect("Failed to parse to_address"),
                entry_point_selector: Felt::from_hex(
                    "0x8bce41827dd5484d80312a2e43bc42a896e3fcf75bf84c2b49339168dfa00a",
                )
                .expect("Failed to parse selector"),
                calldata: vec![
                    // from_address
                    Felt::from_hex("0x422dd5fe05931e677c0dcbb74ea057874ba4035c5d5784ea626200b7cfc702")
                        .expect("Failed to parse from_address"),
                    // payload
                    Felt::from_hex("0x36a44c6cfb107de7ec925d22cb549b7a881439b70d1fc30209728c5340d46f8")
                        .expect("Failed to parse payload"),
                    Felt::from_hex("0x463a5a7d814c754e6c3c10f9de8024b2bdf20eb56ad5168076636a858402d7e")
                        .expect("Failed to parse payload"),
                    Felt::from_hex("0x23b0052e5e47b8d94ef37350a02dba867cef6b2ee2bee6eea363103df04dc18")
                        .expect("Failed to parse payload"),
                    Felt::from_hex("0x98a7d9b8314c0000").expect("Failed to parse payload"),
                    Felt::from_hex("0x0").expect("Failed to parse payload"),
                    Felt::from_hex("0x2").expect("Failed to parse payload"),
                    Felt::from_hex("0x463a5a7d814c754e6c3c10f9de8024b2bdf20eb56ad5168076636a858402d7e")
                        .expect("Failed to parse payload"),
                    Felt::from_hex("0x2b2822").expect("Failed to parse payload"),
                ],
            },
            /* paid_fee_on_l1 */ 1,
        );

        // Create an instance of the struct containing the get_messaging_hash method
        let client = fixture.client;

        // Call the function and check the result
        match client.get_messaging_hash(&event) {
            Ok(hash) => {
                // Replace with the expected hash value
                let event_hash = Felt::from_bytes_be_slice(hash.as_slice()).to_hex_string();
                assert_eq!(
                    event_hash, "0x600b974add9d5406d3d5602b6b2f8beae3b3708a69968f37fa7739524253d8c",
                    "Hash does not match expected value"
                );
            }
            Err(e) => panic!("Function returned an error: {:?}", e),
        }
        Ok(())
    }

    #[rstest]
    #[tokio::test]
    async fn get_last_event_block_number_works_starknet_client(
        #[future] test_fixture: StarknetClientTextFixture,
    ) -> anyhow::Result<()> {
        let fixture = test_fixture.await;

        // sending state updates using the shared account:
        send_state_update(
            &fixture.context.account,
            fixture.context.deployed_appchain_contract_address,
            StateUpdate {
                block_number: Some(99),
                global_root: Felt::from_hex("0xdeadbeef").expect("Should parse valid test hex value"),
                block_hash: Felt::from_hex("0xdeadbeef").expect("Should parse valid test hex value"),
            },
        )
        .await?;

        let last_event_block_number = send_state_update(
            &fixture.context.account,
            fixture.context.deployed_appchain_contract_address,
            StateUpdate {
                block_number: Some(100),
                global_root: Felt::from_hex("0xdeadbeef").expect("Should parse valid test hex value"),
                block_hash: Felt::from_hex("0xdeadbeef").expect("Should parse valid test hex value"),
            },
        )
        .await?;

        poll_on_block_completion(last_event_block_number, fixture.context.account.provider(), 100).await?;

        let latest_event_block_number = fixture.client.get_last_event_block_number().await?;
        assert_eq!(latest_event_block_number, last_event_block_number, "Latest event should have block number 100");

        Ok(())
    }

    #[rstest]
    #[tokio::test]
    async fn get_current_core_contract_state_works_starknet_client(
        #[future] test_fixture: StarknetClientTextFixture,
    ) -> anyhow::Result<()> {
        let fixture = test_fixture.await;

        // sending state updates:
        let block_hash_event = Felt::from_hex("0xdeadbeef").expect("Should parse valid test hex value");
        let global_root_event = Felt::from_hex("0xdeadbeef").expect("Should parse valid test hex value");
        let block_number = send_state_update(
            &fixture.context.account,
            fixture.context.deployed_appchain_contract_address,
            StateUpdate { block_number: Some(100), global_root: global_root_event, block_hash: block_hash_event },
        )
        .await?;
        poll_on_block_completion(block_number, fixture.context.account.provider(), 100).await?;

        let state_update =
            fixture.client.get_current_core_contract_state().await.expect("issue while getting the state");
        assert_eq!(
            state_update,
            StateUpdate { block_number: Some(100), global_root: global_root_event, block_hash: block_hash_event }
        );

        Ok(())
    }

    const RETRY_DELAY: Duration = Duration::from_millis(100);

    pub async fn poll_on_block_completion(
        block_number: u64,
        provider: &JsonRpcClient<HttpTransport>,
        max_retries: u64,
    ) -> anyhow::Result<()> {
        for try_count in 0..=max_retries {
            match provider.get_block_with_tx_hashes(BlockId::Number(block_number)).await {
                Ok(Block(_)) => {
                    return Ok(());
                }
                Ok(PendingBlock(_)) | Err(StarknetError(starknet_core::types::StarknetError::BlockNotFound)) => {
                    if try_count == max_retries {
                        return Err(anyhow::anyhow!("Max retries reached while polling for block {}", block_number));
                    }
                    sleep(RETRY_DELAY).await;
                }
                Err(e) => {
                    return Err(anyhow::anyhow!("Provider error while polling block {}: {}", block_number, e));
                }
            }
        }

        // This line should never be reached due to the return in the loop
        Err(anyhow::anyhow!("Max retries reached while polling for block {}", block_number))
    }
}

#[cfg(test)]
mod starknet_client_messaging_test {
    use super::*;
    use crate::messaging::sync;
    use crate::starknet::test_utils::{
        cancel_messaging_event, fire_messaging_event, get_message_hash_from_cairo, get_test_context,
    };
    use crate::starknet::{StarknetClient, StarknetClientConfig};
    use mc_db::DatabaseService;
    use mp_chain_config::ChainConfig;
    use mp_utils::service::ServiceContext;
    use rstest::{fixture, rstest};
    use std::sync::Arc;
    use std::time::Duration;

    /// This struct holds all commonly used test resources
    pub struct StarknetClientTextFixture {
        pub context: crate::starknet::test_utils::TestContext,
        pub db_service: Arc<DatabaseService>,
        pub starknet_client: StarknetClient,
    }

    #[fixture]
    async fn test_fixture() -> StarknetClientTextFixture {
        let context = get_test_context().await;

        // Set up chain info
        let chain_config = Arc::new(ChainConfig::madara_test());

        // Initialize database service
        let db = Arc::new(DatabaseService::open_for_testing(chain_config.clone()));

        let starknet_client = StarknetClient::new(StarknetClientConfig {
            rpc_url: context.cmd.rpc_url().parse().unwrap(),
            core_contract_address: context.deployed_messaging_contract_address.to_hex_string(),
        })
        .await
        .unwrap();

        // Return all resources bundled together
        StarknetClientTextFixture { context, db_service: db, starknet_client }
    }

    #[rstest]
    #[tokio::test]
    async fn e2e_test_basic_workflow_starknet(#[future] test_fixture: StarknetClientTextFixture) -> anyhow::Result<()> {
        let fixture = test_fixture.await;

        // Start worker handle
        // ==================================
        let worker_handle = {
            let db = Arc::clone(&fixture.db_service);
            let starknet_client = fixture.starknet_client.clone();

            tokio::spawn(async move {
                sync(
                    Arc::new(starknet_client),
                    Arc::clone(db.backend()),
                    Default::default(),
                    ServiceContext::new_for_testing(),
                )
                .await
                .unwrap();
                tracing::debug!("messaging worker stopped");
            })
        };

        // Firing the event
        fire_messaging_event(&fixture.context.account, fixture.context.deployed_messaging_contract_address).await?;
        tokio::time::sleep(Duration::from_secs(10)).await;

        // Assert that the event is well stored in db
        assert!(fixture.db_service.backend().get_pending_message_to_l2(0)?.is_some());

        // Cancelling worker
        worker_handle.abort();

        Ok(())
    }

    #[rstest]
    #[tokio::test]
    async fn e2e_test_message_canceled_starknet(
        #[future] test_fixture: StarknetClientTextFixture,
    ) -> anyhow::Result<()> {
        let fixture = test_fixture.await;

        // Start worker handle
        // ==================================
        let _worker_handle = {
            let db = Arc::clone(&fixture.db_service);
            let starknet_client = fixture.starknet_client.clone();

            tokio::spawn(async move {
                sync(
                    Arc::new(starknet_client),
                    Arc::clone(db.backend()),
                    Default::default(),
                    ServiceContext::new_for_testing(),
                )
                .await
            })
        };

        let message_hash =
            get_message_hash_from_cairo(&fixture.context.account, fixture.context.deployed_appchain_contract_address)
                .await;

        cancel_messaging_event(&fixture.context.account, fixture.context.deployed_appchain_contract_address).await?;
        assert!(fixture.starknet_client.message_to_l2_has_cancel_request(&message_hash.to_bytes_be()).await.unwrap());

        Ok(())
    }
}

#[cfg(test)]
mod starknet_client_event_subscription_test {
    use crate::gas_price::L1BlockMetrics;
    use crate::starknet::test_utils::{get_test_context, send_state_update};
    use crate::starknet::{StarknetClient, StarknetClientConfig};
    use crate::state_update::{state_update_worker, StateUpdate};
    use mc_db::DatabaseService;
    use mp_chain_config::ChainConfig;
    use mp_utils::service::ServiceContext;
    use starknet_types_core::felt::Felt;
    use std::sync::Arc;

    #[tokio::test]
    async fn listen_and_update_state_when_event_fired_starknet_client() -> anyhow::Result<()> {
        let context = get_test_context().await;

        // Setting up the DB and l1 block metrics
        // ================================================
        let chain_config = Arc::new(ChainConfig::madara_test());

        // Initialize database service
        let db = Arc::new(DatabaseService::open_for_testing(chain_config.clone()));

        let starknet_client = StarknetClient::new(StarknetClientConfig {
            rpc_url: context.cmd.rpc_url().parse().unwrap(),
            core_contract_address: context.deployed_appchain_contract_address.to_hex_string(),
        })
        .await?;

        let l1_block_metrics = L1BlockMetrics::register()?;
        let (snd, mut recv) = tokio::sync::watch::channel(None);

        let listen_handle = {
            let db = Arc::clone(&db);
            tokio::spawn(async move {
                state_update_worker(
                    Arc::clone(db.backend()),
                    Arc::new(starknet_client),
                    ServiceContext::new_for_testing(),
                    snd,
                    Arc::new(l1_block_metrics),
                )
                .await
                .expect("Should successfully init state update worker.")
            })
        };

        // Wait for get_initial_state
        recv.changed().await.unwrap();
        assert_eq!(recv.borrow().as_ref().unwrap().block_number, Some(0));

        // Verify the block number
        let block_in_db = db
            .backend()
            .get_l1_last_confirmed_block()
            .expect("Should successfully retrieve the last confirmed block number from the database");
        assert_eq!(block_in_db, Some(0), "Block in DB does not match expected L2 block number");

        // Firing the state update event
        send_state_update(
            &context.account,
            context.deployed_appchain_contract_address,
            StateUpdate {
                block_number: Some(100),
                global_root: Felt::from_hex("0xbeef")?,
                block_hash: Felt::from_hex("0xbeef")?,
            },
        )
        .await?;

        // Wait for changed
        recv.changed().await.unwrap();
        assert_eq!(recv.borrow().as_ref().unwrap().block_number, Some(100));

        // Verify the block number
        let block_in_db = db
            .backend()
            .get_l1_last_confirmed_block()
            .expect("Should successfully retrieve the last confirmed block number from the database");
        assert_eq!(block_in_db, Some(100), "Block in DB does not match expected L2 block number");

        // Abort the worker before ending the test
        listen_handle.abort();

        Ok(())
    }
}
