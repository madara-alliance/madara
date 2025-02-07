use crate::client::{ClientType, SettlementClientTrait};
use crate::error::SettlementClientError;
use crate::gas_price::L1BlockMetrics;
use crate::messaging::CommonMessagingEventData;
use crate::starknet::event::StarknetEventStream;
use crate::state_update::{update_l1, StateUpdate};
use anyhow::anyhow;
use async_trait::async_trait;
use bigdecimal::ToPrimitive;
use mc_db::l1_db::LastSyncedEventBlock;
use mc_db::MadaraBackend;
use mp_utils::service::ServiceContext;
use starknet_core::types::{BlockId, BlockTag, EmittedEvent, EventFilter, FunctionCall};
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

pub mod event;
#[cfg(test)]
pub mod utils;

#[derive(Debug)]
pub struct StarknetClient {
    pub provider: Arc<JsonRpcClient<HttpTransport>>,
    pub l2_core_contract: Felt,
    pub processed_update_state_block: AtomicU64,
}

#[derive(Clone)]
pub struct StarknetClientConfig {
    pub url: Url,
    pub l2_contract_address: Felt,
}

impl Clone for StarknetClient {
    fn clone(&self) -> Self {
        StarknetClient {
            provider: Arc::clone(&self.provider),
            l2_core_contract: self.l2_core_contract,
            processed_update_state_block: AtomicU64::new(self.processed_update_state_block.load(Ordering::Relaxed)),
        }
    }
}

// Add this new implementation block for constructor
impl StarknetClient {
    pub async fn new(config: StarknetClientConfig) -> Result<Self, SettlementClientError> {
        let provider = JsonRpcClient::new(HttpTransport::new(config.url));
        // Check if l2 contract exists
        provider
            .get_class_at(BlockId::Tag(BlockTag::Latest), config.l2_contract_address)
            .await
            .map_err(|e| SettlementClientError::Other(e.into()))?;

        Ok(Self {
            provider: Arc::new(provider),
            l2_core_contract: config.l2_contract_address,
            processed_update_state_block: AtomicU64::new(0), // Keeping this as 0 initially when client is initialized.
        })
    }
}

// TODO : Remove github refs after implementing the zaun imports
// Imp ⚠️ : zaun is not yet updated with latest app chain core contract implementations
//          For this reason we are adding our own call implementations.
#[async_trait]
impl SettlementClientTrait for StarknetClient {
    type Config = StarknetClientConfig;

    fn get_client_type(&self) -> ClientType {
        ClientType::STARKNET
    }

    async fn get_latest_block_number(&self) -> Result<u64, SettlementClientError> {
        self.provider.block_number().await.map_err(|e| SettlementClientError::Other(e.into()))
    }

    async fn get_last_event_block_number(&self) -> Result<u64, SettlementClientError> {
        let latest_block = self.get_latest_block_number().await?;
        // If block on l2 is not greater than or equal to 6000 we will consider the last block to 0.
        let last_block = if latest_block <= 6000 { 0 } else { latest_block - 6000 };
        let last_events = self
            .get_events(
                BlockId::Number(last_block),
                BlockId::Number(latest_block),
                self.l2_core_contract,
                vec![get_selector_from_name("LogStateUpdate").map_err(|e| SettlementClientError::Other(e.into()))?],
            )
            .await?;

        let last_update_state_event =
            last_events.last().ok_or_else(|| SettlementClientError::Other(anyhow!("No event found")))?;
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
        if last_update_state_event.data.len() != 3 {
            return Err(SettlementClientError::InvalidEvent("Event response invalid".into()));
        }

        match last_update_state_event.block_number {
            Some(block_number) => Ok(block_number),
            None => Ok(self.get_latest_block_number().await? + 1),
        }
    }

    async fn get_last_verified_block_number(&self) -> Result<u64, SettlementClientError> {
        let state = self.get_state_call().await?;
        u64::try_from(state[1]).map_err(|e| SettlementClientError::ConversionError(e.to_string()))
    }

    async fn get_last_verified_state_root(&self) -> Result<Felt, SettlementClientError> {
        Ok(self.get_state_call().await?[0])
    }

    async fn get_last_verified_block_hash(&self) -> Result<Felt, SettlementClientError> {
        Ok(self.get_state_call().await?[2])
    }

    async fn get_initial_state(&self) -> Result<StateUpdate, SettlementClientError> {
        let block_number = self.get_last_verified_block_number().await?;
        let block_hash = self.get_last_verified_block_hash().await?;
        let global_root = self.get_last_verified_state_root().await?;

        Ok(StateUpdate { global_root, block_number, block_hash })
    }

    async fn listen_for_update_state_events(
        &self,
        backend: Arc<MadaraBackend>,
        mut ctx: ServiceContext,
        l1_block_metrics: Arc<L1BlockMetrics>,
    ) -> Result<(), SettlementClientError> {
        while let Some(events) = ctx
            .run_until_cancelled(async {
                let latest_block = self.get_latest_block_number().await?;
                let selector =
                    get_selector_from_name("LogStateUpdate").map_err(|e| SettlementClientError::Other(e.into()))?;

                self.get_events(
                    BlockId::Number(latest_block),
                    BlockId::Number(latest_block),
                    self.l2_core_contract,
                    vec![selector],
                )
                .await
            })
            .await
        {
            let events_fetched = events?;
            if let Some(event) = events_fetched.last() {
                let data = event;
                let block_number = data.data[1]
                    .to_u64()
                    .ok_or_else(|| SettlementClientError::ConversionError("Block number conversion failed".into()))?;

                let current_processed = self.processed_update_state_block.load(Ordering::Relaxed);
                if current_processed < block_number {
                    let formatted_event =
                        StateUpdate { block_number, global_root: data.data[0], block_hash: data.data[2] };
                    update_l1(&backend, formatted_event, l1_block_metrics.clone())
                        .map_err(|e| SettlementClientError::Other(e.into()))?;
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

    fn get_messaging_hash(&self, event: &CommonMessagingEventData) -> Result<Vec<u8>, SettlementClientError> {
        Ok(poseidon_hash_many(&self.event_to_felt_array(event)).to_bytes_be().to_vec())
    }

    async fn get_l1_to_l2_message_cancellations(&self, msg_hash: Vec<u8>) -> Result<Felt, SettlementClientError> {
        let call_res = self
            .provider
            .call(
                FunctionCall {
                    contract_address: self.l2_core_contract,
                    entry_point_selector: get_selector_from_name("l1_to_l2_message_cancellations")
                        .map_err(|e| SettlementClientError::Other(e.into()))?,
                    calldata: vec![Felt::from_bytes_be_slice(msg_hash.as_slice())],
                },
                BlockId::Tag(BlockTag::Pending),
            )
            .await
            .map_err(|e| SettlementClientError::Other(e.into()))?;

        if call_res.len() != 2 {
            return Err(SettlementClientError::InvalidResponse(
                "l1_to_l2_message_cancellations should return only 2 values".into(),
            ));
        }
        Ok(call_res[0])
    }

    type StreamType = StarknetEventStream;

    async fn get_messaging_stream(
        &self,
        last_synced_event_block: LastSyncedEventBlock,
    ) -> Result<Self::StreamType, SettlementClientError> {
        let filter = EventFilter {
            from_block: Some(BlockId::Number(last_synced_event_block.block_number)),
            to_block: Some(BlockId::Number(self.get_latest_block_number().await?)),
            address: Some(self.l2_core_contract),
            keys: Some(vec![vec![
                get_selector_from_name("MessageSent").map_err(|e| SettlementClientError::Other(e.into()))?
            ]]),
        };
        Ok(StarknetEventStream::new(self.provider.clone(), filter, Duration::from_secs(1)))
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
                .await
                .map_err(|e| SettlementClientError::Other(e.into()))?;

            event_vec.extend(events.events);
            if let Some(token) = events.continuation_token {
                continuation_token = Some(token);
            } else {
                page_indicator = true;
            }
        }

        Ok(event_vec)
    }

    fn event_to_felt_array(&self, event: &CommonMessagingEventData) -> Vec<Felt> {
        let mut felt_vec = vec![event.from, event.to, event.selector, event.nonce];
        felt_vec.push(Felt::from(event.payload.len()));
        event.payload.clone().into_iter().for_each(|felt| {
            felt_vec.push(felt);
        });

        felt_vec
    }

    pub async fn get_state_call(&self) -> Result<Vec<Felt>, SettlementClientError> {
        let call_res = self
            .provider
            .call(
                FunctionCall {
                    contract_address: self.l2_core_contract,
                    /*
                    GitHub Ref : https://github.com/keep-starknet-strange/piltover/blob/main/src/state/component.cairo#L59
                    Function Call response : (StateRoot, BlockNumber, BlockHash)
                    */
                    entry_point_selector: get_selector_from_name("get_state")
                        .map_err(|e| SettlementClientError::Other(e.into()))?,
                    calldata: vec![],
                },
                BlockId::Tag(BlockTag::Pending),
            )
            .await
            .map_err(|e| SettlementClientError::Other(e.into()))?;
        if call_res.len() != 3 {
            return Err(SettlementClientError::InvalidResponse("Call response invalid !!".into()));
        }
        Ok(call_res)
    }
}

#[cfg(test)]
pub mod starknet_client_tests {
    use crate::client::SettlementClientTrait;
    use crate::starknet::utils::{prepare_starknet_client_test, send_state_update, MADARA_PORT};
    use crate::starknet::{StarknetClient, StarknetClientConfig};
    use crate::state_update::StateUpdate;
    use starknet_accounts::ConnectedAccount;
    use starknet_core::types::BlockId;
    use starknet_core::types::MaybePendingBlockWithTxHashes::{Block, PendingBlock};
    use starknet_providers::jsonrpc::HttpTransport;
    use starknet_providers::ProviderError::StarknetError;
    use starknet_providers::{JsonRpcClient, Provider};
    use starknet_types_core::felt::Felt;
    use std::str::FromStr;
    use std::time::Duration;
    use tokio::time::sleep;
    use url::Url;

    #[tokio::test]
    async fn fail_create_new_client_contract_does_not_exists() -> anyhow::Result<()> {
        prepare_starknet_client_test().await?;
        let starknet_client = StarknetClient::new(StarknetClientConfig {
            url: Url::parse(format!("http://127.0.0.1:{}", MADARA_PORT).as_str())?,
            l2_contract_address: Felt::from_str("0xdeadbeef")?,
        })
        .await;
        assert!(starknet_client.is_err(), "Should fail to create a new client");
        Ok(())
    }

    #[tokio::test]
    async fn create_new_client_contract_exists_starknet_client() -> anyhow::Result<()> {
        let (_, deployed_address, _madara) = prepare_starknet_client_test().await?;
        let starknet_client = StarknetClient::new(StarknetClientConfig {
            url: Url::parse(format!("http://127.0.0.1:{}", MADARA_PORT).as_str())?,
            l2_contract_address: deployed_address,
        })
        .await;
        assert!(starknet_client.is_ok(), "Should not fail to create a new client");
        Ok(())
    }

    #[tokio::test]
    async fn get_last_event_block_number_works_starknet_client() -> anyhow::Result<()> {
        let (account, deployed_address, _madara) = prepare_starknet_client_test().await?;
        let starknet_client = StarknetClient::new(StarknetClientConfig {
            url: Url::parse(format!("http://127.0.0.1:{}", MADARA_PORT).as_str())?,
            l2_contract_address: deployed_address,
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

        poll_on_block_completion(last_event_block_number, account.provider(), 100).await?;

        let latest_event_block_number = starknet_client.get_last_event_block_number().await?;
        assert_eq!(latest_event_block_number, last_event_block_number, "Latest event should have block number 100");
        Ok(())
    }

    #[tokio::test]
    async fn get_last_verified_block_hash_works_starknet_client() -> anyhow::Result<()> {
        let (account, deployed_address, _madara) = prepare_starknet_client_test().await?;
        let starknet_client = StarknetClient::new(StarknetClientConfig {
            url: Url::parse(format!("http://127.0.0.1:{}", MADARA_PORT).as_str())?,
            l2_contract_address: deployed_address,
        })
        .await?;

        // sending state updates :
        let block_hash_event = Felt::from_hex("0xdeadbeef")?;
        let global_root_event = Felt::from_hex("0xdeadbeef")?;
        let block_number = send_state_update(
            &account,
            deployed_address,
            StateUpdate { block_number: 100, global_root: global_root_event, block_hash: block_hash_event },
        )
        .await?;
        poll_on_block_completion(block_number, account.provider(), 100).await?;

        let last_verified_block_hash = starknet_client.get_last_verified_block_hash().await?;
        assert_eq!(last_verified_block_hash, block_hash_event, "Block hash should match");

        Ok(())
    }

    #[tokio::test]
    async fn get_last_state_root_works_starknet_client() -> anyhow::Result<()> {
        let (account, deployed_address, _madara) = prepare_starknet_client_test().await?;
        let starknet_client = StarknetClient::new(StarknetClientConfig {
            url: Url::parse(format!("http://127.0.0.1:{}", MADARA_PORT).as_str())?,
            l2_contract_address: deployed_address,
        })
        .await?;

        // sending state updates :
        let block_hash_event = Felt::from_hex("0xdeadbeef")?;
        let global_root_event = Felt::from_hex("0xdeadbeef")?;
        let block_number = send_state_update(
            &account,
            deployed_address,
            StateUpdate { block_number: 100, global_root: global_root_event, block_hash: block_hash_event },
        )
        .await?;
        poll_on_block_completion(block_number, account.provider(), 100).await?;

        let last_verified_state_root = starknet_client.get_last_verified_state_root().await?;
        assert_eq!(last_verified_state_root, global_root_event, "Last state root should match");

        Ok(())
    }

    #[tokio::test]
    async fn get_last_verified_block_number_works_starknet_client() -> anyhow::Result<()> {
        let (account, deployed_address, _madara) = prepare_starknet_client_test().await?;
        let starknet_client = StarknetClient::new(StarknetClientConfig {
            url: Url::parse(format!("http://127.0.0.1:{}", MADARA_PORT).as_str())?,
            l2_contract_address: deployed_address,
        })
        .await?;

        // sending state updates :
        let data_felt = Felt::from_hex("0xdeadbeef")?;
        let block_number = 100;
        let event_block_number = send_state_update(
            &account,
            deployed_address,
            StateUpdate { block_number, global_root: data_felt, block_hash: data_felt },
        )
        .await?;
        poll_on_block_completion(event_block_number, account.provider(), 100).await?;

        let last_verified_block_number = starknet_client.get_last_verified_block_number().await?;
        assert_eq!(last_verified_block_number, block_number, "Last verified block should match");

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
mod l2_messaging_test {
    use crate::messaging::sync;
    use crate::starknet::utils::{
        cancel_messaging_event, fire_messaging_event, prepare_starknet_client_messaging_test, MadaraProcess,
        StarknetAccount, MADARA_PORT,
    };
    use crate::starknet::{StarknetClient, StarknetClientConfig};
    use mc_db::DatabaseService;
    use mc_mempool::{GasPriceProvider, L1DataProvider, Mempool, MempoolLimits};
    use mp_chain_config::ChainConfig;
    use mp_utils::service::ServiceContext;
    use rstest::{fixture, rstest};
    use starknet_api::core::Nonce;
    use starknet_types_core::felt::Felt;
    use std::sync::Arc;
    use std::time::Duration;
    use tempfile::TempDir;
    use tracing_test::traced_test;
    use url::Url;

    struct TestRunnerStarknet {
        #[allow(dead_code)]
        madara: MadaraProcess, // Not used but needs to stay in scope otherwise it will be dropped
        account: StarknetAccount,
        chain_config: Arc<ChainConfig>,
        db_service: Arc<DatabaseService>,
        deployed_address: Felt,
        starknet_client: StarknetClient,
        mempool: Arc<Mempool>,
    }

    #[fixture]
    async fn setup_test_env_starknet() -> TestRunnerStarknet {
        let (account, deployed_contract_address, madara) = prepare_starknet_client_messaging_test().await.unwrap();

        // Set up chain info
        let chain_config = Arc::new(ChainConfig::madara_test());

        // Set up database paths
        let temp_dir = TempDir::new().expect("issue while creating temporary directory");
        let base_path = temp_dir.path().join("data");
        let backup_dir = Some(temp_dir.path().join("backups"));

        // Initialize database service
        let db = Arc::new(
            DatabaseService::new(&base_path, backup_dir, false, chain_config.clone(), Default::default())
                .await
                .expect("Failed to create database service"),
        );

        let starknet_client = StarknetClient::new(StarknetClientConfig {
            url: Url::parse(format!("http://127.0.0.1:{}", MADARA_PORT).as_str()).unwrap(),
            l2_contract_address: deployed_contract_address,
        })
        .await
        .unwrap();

        let l1_gas_setter = GasPriceProvider::new();
        let l1_data_provider: Arc<dyn L1DataProvider> = Arc::new(l1_gas_setter.clone());

        let mempool = Arc::new(Mempool::new(
            Arc::clone(db.backend()),
            Arc::clone(&l1_data_provider),
            MempoolLimits::for_testing(),
        ));

        TestRunnerStarknet {
            madara,
            account,
            chain_config,
            db_service: db,
            deployed_address: deployed_contract_address,
            starknet_client,
            mempool,
        }
    }

    #[rstest]
    #[traced_test]
    #[tokio::test]
    async fn e2e_test_basic_workflow_starknet(
        #[future] setup_test_env_starknet: TestRunnerStarknet,
    ) -> anyhow::Result<()> {
        // Initial Setup
        // ==================================
        let TestRunnerStarknet {
            madara: _madara,
            account,
            chain_config,
            db_service: db,
            deployed_address: deployed_contract_address,
            starknet_client,
            mempool,
        } = setup_test_env_starknet.await;

        // Start worker handle
        // ==================================
        let worker_handle = {
            let db = Arc::clone(&db);
            tokio::spawn(async move {
                sync(
                    Arc::new(Box::new(starknet_client)),
                    Arc::clone(db.backend()),
                    chain_config.chain_id.clone(),
                    mempool,
                    ServiceContext::new_for_testing(),
                )
                .await
            })
        };

        // Firing the event
        let fire_event_block_number = fire_messaging_event(&account, deployed_contract_address).await?;
        tokio::time::sleep(Duration::from_secs(10)).await;

        // Log asserts
        // ===========
        assert!(logs_contain("fromAddress: \"0x07484e8e3af210b2ead47fa08c96f8d18b616169b350a8b75fe0dc4d2e01d493\""));
        // hash calculated in the contract : 0x210c8d7fdedf3e9d775ba12b12da86ea67878074a21b625e06dac64d5838ad0
        // expecting the same in logs
        assert!(logs_contain("event hash: \"0x210c8d7fdedf3e9d775ba12b12da86ea67878074a21b625e06dac64d5838ad0\""));

        // Assert that the event is well stored in db
        let last_block =
            db.backend().messaging_last_synced_l1_block_with_event().expect("failed to retrieve block").unwrap();
        assert_eq!(last_block.block_number, fire_event_block_number);
        let nonce = Nonce(Felt::from_dec_str("10000000000000000").expect("failed to parse nonce string"));
        assert!(db.backend().has_l1_messaging_nonce(nonce)?);

        // Cancelling worker
        worker_handle.abort();
        Ok(())
    }

    #[rstest]
    #[traced_test]
    #[tokio::test]
    // This test is redundant now as the event poller will not return the same
    // event twice with same nonce that's why added ignore here.
    #[ignore]
    async fn e2e_test_already_processed_event_starknet(
        #[future] setup_test_env_starknet: TestRunnerStarknet,
    ) -> anyhow::Result<()> {
        // Initial Setup
        // ==================================
        let TestRunnerStarknet {
            madara: _madara,
            account,
            chain_config,
            db_service: db,
            deployed_address: deployed_contract_address,
            starknet_client,
            mempool,
        } = setup_test_env_starknet.await;

        // Start worker handle
        // ==================================
        let worker_handle = {
            let db = Arc::clone(&db);
            tokio::spawn(async move {
                sync(
                    Arc::new(Box::new(starknet_client)),
                    Arc::clone(db.backend()),
                    chain_config.chain_id.clone(),
                    mempool,
                    ServiceContext::new_for_testing(),
                )
                .await
            })
        };

        // Firing the event
        let fire_event_block_number = fire_messaging_event(&account, deployed_contract_address).await?;
        tokio::time::sleep(Duration::from_secs(10)).await;

        // Log asserts
        // ===========
        assert!(logs_contain("fromAddress: \"0x07484e8e3af210b2ead47fa08c96f8d18b616169b350a8b75fe0dc4d2e01d493\""));
        // hash calculated in the contract : 0x210c8d7fdedf3e9d775ba12b12da86ea67878074a21b625e06dac64d5838ad0
        // expecting the same in logs
        assert!(logs_contain("event hash: \"0x210c8d7fdedf3e9d775ba12b12da86ea67878074a21b625e06dac64d5838ad0\""));

        // Assert that the event is well stored in db
        let last_block =
            db.backend().messaging_last_synced_l1_block_with_event().expect("failed to retrieve block").unwrap();
        assert_eq!(last_block.block_number, fire_event_block_number);
        let nonce = Nonce(Felt::from_dec_str("10000000000000000").expect("failed to parse nonce string"));
        assert!(db.backend().has_l1_messaging_nonce(nonce)?);

        // Firing the event second time
        fire_messaging_event(&account, deployed_contract_address).await?;
        tokio::time::sleep(Duration::from_secs(15)).await;
        // Assert that the event processed was in last block only not in the latest block.
        assert_eq!(
            last_block.block_number,
            db.backend()
                .messaging_last_synced_l1_block_with_event()
                .expect("failed to retrieve block")
                .unwrap()
                .block_number
        );
        assert!(logs_contain("Event already processed"));

        // Cancelling worker
        worker_handle.abort();
        Ok(())
    }

    #[rstest]
    #[traced_test]
    #[tokio::test]
    async fn e2e_test_message_canceled_starknet(
        #[future] setup_test_env_starknet: TestRunnerStarknet,
    ) -> anyhow::Result<()> {
        // Initial Setup
        // ==================================
        let TestRunnerStarknet {
            madara: _madara,
            account,
            chain_config,
            db_service: db,
            deployed_address: deployed_contract_address,
            starknet_client,
            mempool,
        } = setup_test_env_starknet.await;

        // Start worker handle
        // ==================================
        let worker_handle = {
            let db = Arc::clone(&db);
            tokio::spawn(async move {
                sync(
                    Arc::new(Box::new(starknet_client)),
                    Arc::clone(db.backend()),
                    chain_config.chain_id.clone(),
                    mempool,
                    ServiceContext::new_for_testing(),
                )
                .await
            })
        };

        cancel_messaging_event(&account, deployed_contract_address).await?;
        // Firing cancelled event
        fire_messaging_event(&account, deployed_contract_address).await?;
        tokio::time::sleep(Duration::from_secs(15)).await;

        let last_block =
            db.backend().messaging_last_synced_l1_block_with_event().expect("failed to retrieve block").unwrap();
        assert_eq!(last_block.block_number, 0);
        let nonce = Nonce(Felt::from_dec_str("10000000000000000").expect("failed to parse nonce string"));
        // cancelled message nonce should be inserted to avoid reprocessing
        assert!(db.backend().has_l1_messaging_nonce(nonce).unwrap());
        assert!(logs_contain("Message was cancelled in block at timestamp: 0x66b4f105"));

        // Cancelling worker
        worker_handle.abort();
        Ok(())
    }
}

#[cfg(test)]
mod starknet_client_event_subscription_test {
    use crate::gas_price::L1BlockMetrics;
    use crate::starknet::event::StarknetEventStream;
    use crate::starknet::utils::{prepare_starknet_client_test, send_state_update, MADARA_PORT};
    use crate::starknet::{StarknetClient, StarknetClientConfig};
    use crate::state_update::{state_update_worker, StateUpdate};
    use mc_db::DatabaseService;
    use mp_chain_config::ChainConfig;
    use mp_utils::service::ServiceContext;
    use rstest::rstest;
    use starknet_types_core::felt::Felt;
    use std::str::FromStr;
    use std::sync::Arc;
    use std::time::Duration;
    use tempfile::TempDir;
    use url::Url;

    #[rstest]
    #[tokio::test]
    async fn listen_and_update_state_when_event_fired_starknet_client() -> anyhow::Result<()> {
        // Setting up the DB and l1 block metrics
        // ================================================

        let chain_info = Arc::new(ChainConfig::madara_test());

        // Set up database paths
        let temp_dir = TempDir::new().expect("issue while creating temporary directory");
        let base_path = temp_dir.path().join("data");
        let backup_dir = Some(temp_dir.path().join("backups"));

        // Initialize database service
        let db = Arc::new(
            DatabaseService::new(&base_path, backup_dir, false, chain_info.clone(), Default::default())
                .await
                .expect("Failed to create database service"),
        );

        // Making Starknet client and start worker
        // ================================================
        let (account, deployed_address, _madara) = prepare_starknet_client_test().await?;

        let starknet_client = StarknetClient::new(StarknetClientConfig {
            url: Url::parse(format!("http://127.0.0.1:{}", MADARA_PORT).as_str())?,
            l2_contract_address: deployed_address,
        })
        .await?;

        let l1_block_metrics = L1BlockMetrics::register()?;

        let listen_handle = {
            let db = Arc::clone(&db);
            tokio::spawn(async move {
                state_update_worker::<StarknetClientConfig, StarknetEventStream>(
                    Arc::clone(db.backend()),
                    Arc::new(Box::new(starknet_client)),
                    ServiceContext::new_for_testing(),
                    Arc::new(l1_block_metrics),
                )
                .await
                .expect("Failed to init state update worker.")
            })
        };

        // Firing the state update event
        send_state_update(
            &account,
            deployed_address,
            StateUpdate {
                block_number: 100,
                global_root: Felt::from_str("0xbeef")?,
                block_hash: Felt::from_str("0xbeef")?,
            },
        )
        .await?;

        // Wait for this update to be registered in the DB. Approx 10 secs
        tokio::time::sleep(Duration::from_secs(10)).await;

        // Verify the block number
        let block_in_db =
            db.backend().get_l1_last_confirmed_block().expect("Failed to get L2 last confirmed block number");

        listen_handle.abort();
        assert_eq!(block_in_db, Some(100), "Block in DB does not match expected L3 block number");
        Ok(())
    }
}
