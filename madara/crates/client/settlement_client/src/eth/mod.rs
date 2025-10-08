use crate::client::{ClientType, SettlementLayerProvider};
use crate::error::SettlementClientError;
use crate::eth::event::EthereumEventStream;
use crate::eth::StarknetCoreContract::{LogMessageToL2, StarknetCoreContractInstance};
use crate::messaging::MessageToL2WithMetadata;
use crate::state_update::{StateUpdate, StateUpdateWorker};
use crate::utils::convert_log_state_update;
use alloy::eips::{BlockId, BlockNumberOrTag};
use alloy::primitives::{keccak256, Address, B256, I256, U256};
use alloy::providers::{Provider, ProviderBuilder, ReqwestProvider, RootProvider};
use alloy::rpc::types::Filter;
use alloy::sol;
use alloy::sol_types::SolValue;
use alloy::transports::http::{Client, Http};
use async_trait::async_trait;
use bitvec::macros::internal::funty::Fundamental;
use error::EthereumClientError;
use futures::stream::BoxStream;
use futures::StreamExt;
use mp_convert::{FeltExt, ToFelt};
use mp_transactions::L1HandlerTransactionWithFee;
use mp_utils::service::ServiceContext;
use std::str::FromStr;
use std::sync::Arc;
use url::Url;

pub mod error;
pub mod event;

// abi taken from: https://etherscan.io/address/0x6e0acfdc3cf17a7f99ed34be56c3dfb93f464e24#code
// The official starknet core contract ^
sol!(
    #[sol(rpc)]
    #[derive(Debug)]
    StarknetCoreContract,
    "src/eth/starknet_core.json"
);

pub struct EthereumClient {
    pub provider: Arc<ReqwestProvider>,
    pub l1_core_contract: StarknetCoreContractInstance<Http<Client>, RootProvider<Http<Client>>>,
}

#[derive(Clone)]
pub struct EthereumClientConfig {
    pub rpc_url: Url,
    pub core_contract_address: String,
}

impl Clone for EthereumClient {
    fn clone(&self) -> Self {
        EthereumClient { provider: Arc::clone(&self.provider), l1_core_contract: self.l1_core_contract.clone() }
    }
}

impl EthereumClient {
    pub async fn new(config: EthereumClientConfig) -> Result<Self, SettlementClientError> {
        let provider = ProviderBuilder::new().on_http(config.rpc_url);
        let core_contract_address =
            Address::from_str(&config.core_contract_address).map_err(|e| -> SettlementClientError {
                EthereumClientError::Conversion(format!("Invalid core contract address: {e}")).into()
            })?;
        // Check if contract exists
        if !provider
            .get_code_at(core_contract_address)
            .await
            .map_err(|e| -> SettlementClientError { EthereumClientError::Rpc(e.to_string()).into() })?
            .is_empty()
        {
            let contract = StarknetCoreContract::new(core_contract_address, provider.clone());
            Ok(Self { provider: Arc::new(provider), l1_core_contract: contract })
        } else {
            Err(SettlementClientError::Ethereum(EthereumClientError::Contract(
                "Core contract not found at given address".into(),
            )))
        }
    }
}

const HISTORY_SIZE: usize = 300; // Number of blocks to use for gas price calculation (approx. 1 hour at 12 sec block time)

#[async_trait]
impl SettlementLayerProvider for EthereumClient {
    fn get_client_type(&self) -> ClientType {
        ClientType::Eth
    }

    /// Retrieves the latest Ethereum block number
    async fn get_latest_block_number(&self) -> Result<u64, SettlementClientError> {
        self.provider
            .get_block_number()
            .await
            .map(|n| n.as_u64())
            .map_err(|e| -> SettlementClientError { EthereumClientError::Rpc(e.to_string()).into() })
    }

    /// Get the block number of the last occurrence of the LogStateUpdate event.
    async fn get_last_event_block_number(&self) -> Result<u64, SettlementClientError> {
        let latest_block = self.get_latest_block_number().await?;

        let filter = Filter::new().to_block(latest_block).address(*self.l1_core_contract.address());

        let logs = self
            .provider
            .get_logs(&filter)
            .await
            .map_err(|e| -> SettlementClientError { EthereumClientError::Rpc(e.to_string()).into() })?;

        let latest_logs =
            logs.into_iter().rev().map(|log| log.log_decode::<StarknetCoreContract::LogStateUpdate>()).next();

        match latest_logs {
            Some(Ok(log)) => log
                .block_number
                .ok_or_else(|| -> SettlementClientError { EthereumClientError::MissingField("block_number").into() }),
            Some(Err(e)) => Err(SettlementClientError::Ethereum(EthereumClientError::Contract(e.to_string()))),
            None => Err(SettlementClientError::Ethereum(EthereumClientError::EventProcessing {
                message: format!("no LogStateUpdate event found in block range [None, {}]", latest_block),
                block_number: latest_block,
            })),
        }
    }

    async fn get_current_core_contract_state(&self) -> Result<StateUpdate, SettlementClientError> {
        // Get the latest block_n first, to guard against the case when the contract state changed in between the calls following calls.
        let latest_block_n = self.get_latest_block_number().await?;

        let block_number =
            self.l1_core_contract.stateBlockNumber().block(BlockId::number(latest_block_n)).call().await.map_err(
                |e| -> SettlementClientError {
                    EthereumClientError::Contract(format!("Failed to get state block number: {e:#}")).into()
                },
            )?;
        // when the block 0 is not settled yet, this should be prev block number, this would be the output from the snos as well while
        // executing the block 0.
        // link: https://github.com/starkware-libs/cairo-lang/blob/master/src/starkware/starknet/solidity/StarknetState.sol#L32
        let block_number: Option<u64> = if block_number._0 == I256::MINUS_ONE {
            None // initial contract state
        } else {
            Some(block_number._0.as_u64())
        };

        let global_root =
            self.l1_core_contract.stateRoot().block(BlockId::number(latest_block_n)).call().await.map_err(
                |e| -> SettlementClientError {
                    EthereumClientError::Contract(format!("Failed to get state root: {e:#}")).into()
                },
            )?;
        let global_root = global_root._0.to_felt();

        let block_hash =
            self.l1_core_contract.stateBlockHash().block(BlockId::number(latest_block_n)).call().await.map_err(
                |e| -> SettlementClientError {
                    EthereumClientError::Contract(format!("Failed to get state block number: {e:#}")).into()
                },
            )?;
        let block_hash = block_hash._0.to_felt();

        Ok(StateUpdate { global_root, block_number, block_hash })
    }

    /// Listen for state update events from the L1 core contract and process them
    ///
    /// This function runs a blocking loop that continuously polls for new state update events.
    /// It will run until the context is cancelled. Each event is processed and used to update
    /// the L1 state in the backend database.
    ///
    /// # Note
    /// This is a long-running function that blocks the current task until cancelled.
    async fn listen_for_update_state_events(
        &self,
        mut ctx: ServiceContext,
        worker: StateUpdateWorker,
    ) -> Result<(), SettlementClientError> {
        let event_filter = self.l1_core_contract.event_filter::<StarknetCoreContract::LogStateUpdate>();

        let mut event_stream = match ctx.run_until_cancelled(event_filter.watch()).await {
            Some(res) => res
                .map_err(|e| -> SettlementClientError {
                    EthereumClientError::EventStream { message: format!("Failed to watch events: {}", e) }.into()
                })?
                .into_stream(),
            None => return Ok(()),
        };

        // Process events in a loop until the context is cancelled
        while let Some(Some(event_result)) = ctx.run_until_cancelled(event_stream.next()).await {
            let log = event_result.map_err(|e| -> SettlementClientError {
                EthereumClientError::EventStream { message: format!("Failed to process event: {e:#}") }.into()
            })?;

            let format_event = convert_log_state_update(log.0.clone()).map_err(|e| -> SettlementClientError {
                EthereumClientError::StateUpdate { message: format!("Failed to convert log state update: {e:#}") }
                    .into()
            })?;

            worker.update_state(format_event).map_err(|e| -> SettlementClientError {
                EthereumClientError::StateUpdate { message: format!("Failed to update L1 state: {e:#}") }.into()
            })?;
        }

        Ok(())
    }

    async fn get_gas_prices(&self) -> Result<(u128, u128), SettlementClientError> {
        let block_number = self.get_latest_block_number().await?;
        let fee_history = self
            .provider
            .get_fee_history(HISTORY_SIZE as u64, BlockNumberOrTag::Number(block_number), &[])
            .await
            .map_err(|e| -> SettlementClientError {
                EthereumClientError::GasPriceCalculation {
                    message: format!("Failed to get fee history for block {}: {}", block_number, e),
                }
                .into()
            })?;

        // Calculate average blob base fee from recent blocks
        // We use reverse iteration and take() to handle cases where the RPC might return
        // more or fewer elements than requested, ensuring we use at most HISTORY_SIZE blocks
        // for a more stable and representative average gas price
        let avg_blob_base_fee = fee_history
            .base_fee_per_blob_gas
            .iter()
            .rev()
            .take(HISTORY_SIZE)
            .sum::<u128>()
            .checked_div(fee_history.base_fee_per_blob_gas.len() as u128)
            .unwrap_or(0);

        let eth_gas_price = fee_history.base_fee_per_gas.last().ok_or_else(|| -> SettlementClientError {
            EthereumClientError::MissingField("base_fee_per_gas in fee history response").into()
        })?;

        Ok((*eth_gas_price, avg_blob_base_fee))
    }

    fn calculate_message_hash(&self, event: &L1HandlerTransactionWithFee) -> Result<Vec<u8>, SettlementClientError> {
        let from = event.tx.calldata[0];
        let from_address_start_index = from.to_bytes_be().as_slice().len().saturating_sub(20);
        // encoding used here is taken from: https://docs.starknet.io/architecture-and-concepts/network-architecture/messaging-mechanism/#l1_l2_message_structure
        let data = (
            [0u8; 12],
            Address::from_slice(&from.to_bytes_be().as_slice()[from_address_start_index..]),
            event.tx.contract_address.to_u256(),
            U256::from(event.tx.nonce),
            event.tx.entry_point_selector.to_u256(),
            U256::from(event.tx.calldata.len() - 1),
            event.tx.calldata[1..].iter().map(FeltExt::to_u256).collect::<Vec<_>>(),
        );
        Ok(keccak256(data.abi_encode_packed()).as_slice().to_vec())
    }

    /// Get cancellation status of an L1 to L2 message
    ///
    /// This function query the core contract to know if a L1->L2 message has been cancelled
    /// # Arguments
    ///
    /// - msg_hash : Hash of L1 to L2 message
    ///
    /// # Return
    ///
    /// - `true` if there is a cancellation request for this message to l2.
    /// - An Error if the call fail
    async fn message_to_l2_has_cancel_request(&self, msg_hash: &[u8]) -> Result<bool, SettlementClientError> {
        let cancellation_timestamp =
            self.l1_core_contract.l1ToL2MessageCancellations(B256::from_slice(msg_hash)).call().await.map_err(
                |e| -> SettlementClientError {
                    EthereumClientError::L1ToL2Messaging {
                        message: format!("Failed to check message cancellation status: {}", e),
                    }
                    .into()
                },
            )?;

        Ok(!cancellation_timestamp._0.is_zero())
    }

    /// Get cancellation status of an L1 to L2 message
    ///
    /// This function query the core contract to know if a L1->L2 exists in the contract. This is useful
    /// for processing old L1 to L2 messages, because when a message is fully cancelled we will find its old event
    /// but no associated message or cancellation request.
    ///
    /// # Arguments
    ///
    /// - msg_hash : Hash of L1 to L2 message
    ///
    /// # Return
    ///
    /// - `true` if the message exists in the core contract
    /// - An Error if the call fail
    async fn message_to_l2_is_pending(&self, msg_hash: &[u8]) -> Result<bool, SettlementClientError> {
        tracing::debug!("Calling l1ToL2Messages");
        let cancellation_timestamp =
            self.l1_core_contract.l1ToL2Messages(B256::from_slice(msg_hash)).call().await.map_err(
                |e| -> SettlementClientError {
                    EthereumClientError::L1ToL2Messaging {
                        message: format!("Failed to check that message exists, status: {}", e),
                    }
                    .into()
                },
            )?;

        tracing::debug!("Returned");
        Ok(cancellation_timestamp._0 != U256::ZERO)
    }

    async fn get_block_n_timestamp(&self, l1_block_n: u64) -> Result<u64, SettlementClientError> {
        let block = self
            .provider
            .get_block(
                BlockId::Number(BlockNumberOrTag::Number(l1_block_n)),
                alloy::rpc::types::BlockTransactionsKind::Hashes,
            )
            .await
            .map_err(|e| -> SettlementClientError {
                EthereumClientError::ArchiveRequired(format!("Could not get block timestamp: {}", e)).into()
            })?
            .ok_or_else(|| -> SettlementClientError {
                EthereumClientError::ArchiveRequired(format!("Cannot find block: {}", l1_block_n)).into()
            })?;

        Ok(block.header.timestamp)
    }

    async fn messages_to_l2_stream(
        &self,
        from_l1_block_n: u64,
    ) -> Result<BoxStream<'static, Result<MessageToL2WithMetadata, SettlementClientError>>, SettlementClientError> {
        let filter = self.l1_core_contract.event_filter::<LogMessageToL2>();
        let event_stream =
            filter.from_block(from_l1_block_n).to_block(BlockNumberOrTag::Finalized).watch().await.map_err(
                |e| -> SettlementClientError {
                    EthereumClientError::ArchiveRequired(format!(
                        "Could not fetch events, archive node may be required: {}",
                        e
                    ))
                    .into()
                },
            )?;

        Ok(EthereumEventStream::new(event_stream).boxed())
    }
}

#[cfg(test)]
pub mod eth_client_getter_test {
    use super::*;
    use alloy::primitives::{I256, U256};
    use httpmock::Method::POST;
    use httpmock::MockServer;
    use serde_json::json;
    use std::sync::Arc;
    use tokio;

    // Constants remain the same
    // transaction on ethereum mainnet:https://etherscan.io/tx/0xcadb202495cd8adba0d9b382caff907abf755cd42633d23c4988f875f2995d81
    // mapping of the l2<>l1 block number: https://voyager.online/l1/tx/0xcadb202495cd8adba0d9b382caff907abf755cd42633d23c4988f875f2995d81
    const L1_BLOCK_NUMBER: u64 = 20395662;
    const CORE_CONTRACT_ADDRESS: &str = "0xc662c410C0ECf747543f5bA90660f6ABeBD9C8c4";
    // block on the starknet mainnet: https://voyager.online/block/0x13ec4dc67608729b9169a916ceec3a1bf2e940082211253fc8f9dbf2c594ff8
    const L2_BLOCK_NUMBER: u64 = 662703;
    const L2_BLOCK_HASH: &str = "563216050958639290223177746678863910249919294431961492885921903486585884664";
    const L2_STATE_ROOT: &str = "1456190284387746219409791261254265303744585499659352223397867295223408682130";

    pub fn get_anvil_url() -> String {
        std::env::var("ANVIL_URL").unwrap_or_else(|_| {
            panic!(
                "ANVIL_URL environment variable not set. Make sure anvil is running in fork mode from block number {}",
                L1_BLOCK_NUMBER
            )
        })
    }

    pub fn create_ethereum_client(url: String) -> EthereumClient {
        let rpc_url: Url = url.parse().expect("issue while parsing URL");
        let provider = ProviderBuilder::new().on_http(rpc_url.clone());
        let address = Address::parse_checksummed(CORE_CONTRACT_ADDRESS, None).unwrap();
        let contract = StarknetCoreContract::new(address, provider.clone());
        EthereumClient { provider: Arc::new(provider), l1_core_contract: contract }
    }

    #[tokio::test]
    async fn fail_create_new_client_invalid_core_contract() {
        // Sepolia core contract instead of mainnet
        const INVALID_CORE_CONTRACT_ADDRESS: &str = "0xE2Bb56ee936fd6433DC0F6e7e3b8365C906AA057";

        let rpc_url: Url = get_anvil_url().parse().unwrap();
        let core_contract_address = Address::parse_checksummed(INVALID_CORE_CONTRACT_ADDRESS, None)
            .expect("Should parse valid Ethereum address in test");
        let ethereum_client_config =
            EthereumClientConfig { rpc_url, core_contract_address: core_contract_address.to_string() };
        let new_client_result = EthereumClient::new(ethereum_client_config).await;
        assert!(new_client_result.is_err(), "EthereumClient::new should fail with an invalid core contract address");
    }

    #[tokio::test]
    async fn get_latest_block_number_works() {
        let eth_client = create_ethereum_client(get_anvil_url());
        let block_number =
            eth_client.provider.get_block_number().await.expect("issue while fetching the block number").as_u64();
        assert_eq!(block_number, L1_BLOCK_NUMBER, "provider unable to get the correct block number");
    }

    #[tokio::test]
    async fn get_last_event_block_number_works() {
        let eth_client = create_ethereum_client(get_anvil_url());
        let block_number = eth_client
            .get_last_event_block_number()
            .await
            .expect("issue while getting the last block number with given event");
        assert_eq!(block_number, L1_BLOCK_NUMBER, "block number with given event not matching");
    }

    #[tokio::test]
    async fn get_current_core_contract_state_works() {
        let eth_client = create_ethereum_client(get_anvil_url());
        let state_update = eth_client.get_current_core_contract_state().await.expect("issue while getting the state");
        assert_eq!(
            state_update,
            StateUpdate {
                block_number: Some(L2_BLOCK_NUMBER),
                global_root: U256::from_str_radix(L2_STATE_ROOT, 10)
                    .expect("Should parse the predefined L2 state root")
                    .to_felt(),
                block_hash: U256::from_str_radix(L2_BLOCK_HASH, 10)
                    .expect("Should parse the predefined L2 block hash")
                    .to_felt()
            }
        )
    }

    #[tokio::test]
    async fn test_get_current_core_contract_state_with_initial_block_number() {
        // Set up mock server to simulate L1 node
        let server = MockServer::start();

        // Mock the RPC endpoint to return -1 as int256 (all f's in hex)
        let rpc_mock = server.mock(|when, then| {
            when.method(POST).path("/");
            then.status(200).header("content-type", "application/json").json_body(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
            }));
        });

        // Set up client with mock server
        let config = EthereumClientConfig {
            rpc_url: server.url("/").parse().unwrap(),
            core_contract_address: Address::parse_checksummed("0xc662c410C0ECf747543f5bA90660f6ABeBD9C8c4", None)
                .unwrap()
                .to_string(),
        };

        let provider = ProviderBuilder::new().on_http(config.rpc_url);
        let contract = StarknetCoreContract::new(config.core_contract_address.parse().unwrap(), provider.clone());
        let eth_client = EthereumClient { provider: Arc::new(provider), l1_core_contract: contract };

        // Call contract and verify we get -1 as int256
        let block_number = eth_client
            .l1_core_contract
            .stateBlockNumber()
            .block(BlockId::number(10000))
            .call()
            .await
            .map_err(|e| -> SettlementClientError {
                EthereumClientError::Contract(format!("Failed to get state block number: {e:#}")).into()
            })
            .unwrap();

        assert_eq!(block_number._0, I256::MINUS_ONE);

        // Verify that converting -1 to u64 returns None
        let block_number: Option<u64> = block_number._0.try_into().ok();
        assert!(block_number.is_none());

        // Verify mock was called
        rpc_mock.assert();
    }
}

#[cfg(test)]
mod l1_messaging_tests {
    use self::DummyContract::DummyContractInstance;
    use crate::client::SettlementLayerProvider;
    use crate::eth::{EthereumClient, StarknetCoreContract};
    use crate::messaging::sync;
    use alloy::{
        hex::FromHex,
        node_bindings::{Anvil, AnvilInstance},
        primitives::{Address, U256},
        providers::{ProviderBuilder, RootProvider},
        sol,
        transports::http::{Client, Http},
    };
    use mc_db::MadaraBackend;
    use mp_chain_config::ChainConfig;
    use mp_transactions::{L1HandlerTransaction, L1HandlerTransactionWithFee};
    use mp_utils::service::ServiceContext;
    use rstest::*;
    use starknet_types_core::felt::Felt;
    use std::{sync::Arc, time::Duration};
    use tracing_test::traced_test;
    use url::Url;

    struct TestRunner {
        #[allow(dead_code)]
        anvil: AnvilInstance, // Not used but needs to stay in scope otherwise it will be dropped
        db_service: Arc<MadaraBackend>,
        dummy_contract: DummyContractInstance<Http<Client>, RootProvider<Http<Client>>>,
        eth_client: EthereumClient,
    }

    // LogMessageToL2 from https://etherscan.io/tx/0x21980d6674d33e50deee43c6c30ef3b439bd148249b4539ce37b7856ac46b843
    //
    // To obtain the bytecode for testing:
    // 1. Copy the contract code to Remix IDE (https://remix.ethereum.org/)
    // 2. Add the appropriate SPDX license and pragma declaration:
    //    SPDX-License-Identifier: GPL-3.0
    //    pragma solidity >=0.7.0 <0.9.0;
    // 3. Define your contract: contract DummyContract { ... } (this is the contract we are testing)
    // 4. Compile the contract in Remix
    // 5. Navigate to the "Compilation Details" section and copy the bytecode
    sol!(
        #[derive(Debug)]
        #[sol(rpc, bytecode="6080604052348015600e575f5ffd5b506108e18061001c5f395ff3fe608060405234801561000f575f5ffd5b5060043610610055575f3560e01c80634185df151461005957806377c7d7a91461006357806390985ef9146100935780639be446bf146100b1578063af56443a146100e1575b5f5ffd5b6100616100fd565b005b61007d60048036038101906100789190610503565b610176565b60405161008a9190610546565b60405180910390f35b61009b61019b565b6040516100a8919061056e565b60405180910390f35b6100cb60048036038101906100c69190610503565b61020c565b6040516100d89190610546565b60405180910390f35b6100fb60048036038101906100f691906105bc565b610238565b005b5f610106610253565b905080604001518160200151825f015173ffffffffffffffffffffffffffffffffffffffff167fdb80dd488acf86d17c747445b0eabb5d57c541d3bd7b6b87af987858e5066b2b846060015185608001518660a0015160405161016b9392919061069e565b60405180910390a450565b5f5f610180610253565b905060018160a001516101939190610707565b915050919050565b5f5f6101a5610253565b9050805f015173ffffffffffffffffffffffffffffffffffffffff1681602001518260800151836040015184606001515185606001516040516020016101f0969594939291906107e6565b6040516020818303038152906040528051906020012091505090565b5f5f5f9054906101000a900460ff16610225575f61022b565b6366b4f1055b63ffffffff169050919050565b805f5f6101000a81548160ff02191690831515021790555050565b61025b610485565b5f73ae0ee0a63a2ce6baeeffe56e7714fb4efe48d41990505f7f073314940630fd6dcda0d772d4c972c4e0a9946bef9dabf4ef84eda8ef542b8290505f7f01b64b1b3b690b43b9b514fb81377518f4039cd3e4f4914d8a6bdf01d679fb1990505f600767ffffffffffffffff8111156102d7576102d6610851565b5b6040519080825280602002602001820160405280156103055781602001602082028036833780820191505090505b5090506060815f8151811061031d5761031c61087e565b5b60200260200101818152505062195091816001815181106103415761034061087e565b5b60200260200101818152505065231594f0c7ea816002815181106103685761036761087e565b5b60200260200101818152505060058160038151811061038a5761038961087e565b5b60200260200101818152505062455448816004815181106103ae576103ad61087e565b5b60200260200101818152505073bdb193c166cfb7be2e51711c5648ebeef94063bb816005815181106103e3576103e261087e565b5b6020026020010181815250507e7d79cd86ba27a2508a9ca55c8b3474ca082bc5173d0467824f07a32e9db888816006815181106104235761042261087e565b5b6020026020010181815250505f5f90505f5f90506040518060c001604052808773ffffffffffffffffffffffffffffffffffffffff16815260200186815260200185815260200184815260200183815260200182815250965050505050505090565b6040518060c001604052805f73ffffffffffffffffffffffffffffffffffffffff1681526020015f81526020015f8152602001606081526020015f81526020015f81525090565b5f5ffd5b5f819050919050565b6104e2816104d0565b81146104ec575f5ffd5b50565b5f813590506104fd816104d9565b92915050565b5f60208284031215610518576105176104cc565b5b5f610525848285016104ef565b91505092915050565b5f819050919050565b6105408161052e565b82525050565b5f6020820190506105595f830184610537565b92915050565b610568816104d0565b82525050565b5f6020820190506105815f83018461055f565b92915050565b5f8115159050919050565b61059b81610587565b81146105a5575f5ffd5b50565b5f813590506105b681610592565b92915050565b5f602082840312156105d1576105d06104cc565b5b5f6105de848285016105a8565b91505092915050565b5f81519050919050565b5f82825260208201905092915050565b5f819050602082019050919050565b6106198161052e565b82525050565b5f61062a8383610610565b60208301905092915050565b5f602082019050919050565b5f61064c826105e7565b61065681856105f1565b935061066183610601565b805f5b83811015610691578151610678888261061f565b975061068383610636565b925050600181019050610664565b5085935050505092915050565b5f6060820190508181035f8301526106b68186610642565b90506106c56020830185610537565b6106d26040830184610537565b949350505050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f6107118261052e565b915061071c8361052e565b9250828201905080821115610734576107336106da565b5b92915050565b5f819050919050565b61075461074f8261052e565b61073a565b82525050565b5f81905092915050565b61076d8161052e565b82525050565b5f61077e8383610764565b60208301905092915050565b5f610794826105e7565b61079e818561075a565b93506107a983610601565b805f5b838110156107d95781516107c08882610773565b97506107cb83610636565b9250506001810190506107ac565b5085935050505092915050565b5f6107f18289610743565b6020820191506108018288610743565b6020820191506108118287610743565b6020820191506108218286610743565b6020820191506108318285610743565b602082019150610841828461078a565b9150819050979650505050505050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52604160045260245ffd5b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffdfea26469706673582212205448eb5ef94491cb6d6cc96c3aaffcbbdfef4f000aa8dee8729c5adb0732bbaa64736f6c634300081e0033")]
        contract DummyContract {
            bool isCanceled;
            event LogMessageToL2(address indexed _fromAddress, uint256 indexed _toAddress, uint256 indexed _selector, uint256[] payload, uint256 nonce, uint256 fee);

            struct MessageData {
                address fromAddress;
                uint256 toAddress;
                uint256 selector;
                uint256[] payload;
                uint256 nonce;
                uint256 fee;
            }

            function getMessageData() internal pure returns (MessageData memory) {
                address fromAddress = address(993696174272377493693496825928908586134624850969);
                uint256 toAddress = 3256441166037631918262930812410838598500200462657642943867372734773841898370;
                uint256 selector = 774397379524139446221206168840917193112228400237242521560346153613428128537;
                uint256[] memory payload = new uint256[](7);
                payload[0] = 96;
                payload[1] = 1659025;
                payload[2] = 38575600093162;
                payload[3] = 5;
                payload[4] = 4543560;
                payload[5] = 1082959358903034162641917759097118582889062097851;
                payload[6] = 221696535382753200248526706088340988821219073423817576256483558730535647368;
                uint256 nonce = 0;
                uint256 fee = 0;

                return MessageData(fromAddress, toAddress, selector, payload, nonce, fee);
            }

            function fireEvent() public {
                MessageData memory data = getMessageData();
                emit LogMessageToL2(data.fromAddress, data.toAddress, data.selector, data.payload, data.nonce, data.fee);
            }

            function l1ToL2MessageCancellations(bytes32 msgHash) external view returns (uint256) {
                return isCanceled ? 1723134213 : 0;
            }

            function l1ToL2Messages(bytes32 msgHash) external view returns (uint256) {
                MessageData memory data = getMessageData();
                return data.fee + 1;
            }

            function setIsCanceled(bool value) public {
                isCanceled = value;
            }

            function getL1ToL2MsgHash() external pure returns (bytes32) {
                MessageData memory data = getMessageData();
                return keccak256(
                    abi.encodePacked(
                        uint256(uint160(data.fromAddress)),
                        data.toAddress,
                        data.nonce,
                        data.selector,
                        data.payload.length,
                        data.payload
                    )
                );
            }
        }
    );

    /// Common setup for tests
    ///
    /// This test performs the following steps:
    /// 1. Sets up test environemment
    /// 2. Starts worker
    /// 3. Fires a Message event from the dummy contract
    /// 4. Waits for event to be processed
    /// 5. Assert that the worker handle the event with correct data
    /// 6. Assert that the hash computed by the worker is correct
    /// 7. Assert that the tx is succesfully submited
    /// 8. Assert that the event is successfully pushed to the db
    /// 9. TODO : Assert that the tx was correctly executed
    ///
    /// TODO: Test more cases:
    /// - Nonce 1 arrives first and is labeled as Pending
    /// - Nonce 1 arrives first, then Zero and are correctly executed
    #[fixture]
    async fn setup_test_env() -> TestRunner {
        // Start Anvil instance
        let anvil = Anvil::new().block_time(1).chain_id(1337).try_spawn().expect("failed to spawn anvil instance");
        let chain_config = Arc::new(ChainConfig::madara_test());
        // Initialize database service
        let db = MadaraBackend::open_for_testing(chain_config.clone());

        // Set up provider
        let rpc_url: Url = anvil.endpoint().parse().expect("issue while parsing");
        let provider = ProviderBuilder::new().on_http(rpc_url);

        // Set up dummy contract
        let contract = DummyContract::deploy(provider.clone()).await.unwrap();

        let core_contract = StarknetCoreContract::new(*contract.address(), provider.clone());

        let eth_client =
            EthereumClient { provider: Arc::new(provider.clone()), l1_core_contract: core_contract.clone() };

        TestRunner { anvil, db_service: db, dummy_contract: contract, eth_client }
    }

    /// Test the basic workflow of l1 -> l2 messaging
    ///
    /// This test performs the following steps:
    /// 1. Sets up test environemment
    /// 2. Starts worker
    /// 3. Fires a Message event from the dummy contract
    /// 4. Waits for event to be processed
    /// 5. Assert that the worker handle the event with correct data
    /// 6. Assert that the hash computed by the worker is correct
    /// 7. TODO : Assert that the tx is succesfully submited
    /// 8. Assert that the event is successfully pushed to the db
    /// 9. TODO : Assert that the tx was correctly executed
    #[rstest]
    #[traced_test]
    #[tokio::test]
    async fn e2e_test_basic_workflow(#[future] setup_test_env: TestRunner) {
        let TestRunner { db_service: db, dummy_contract: contract, eth_client, anvil: _anvil } = setup_test_env.await;

        // Start worker handle
        let worker_handle = {
            let db = Arc::clone(&db);
            tokio::spawn(async move {
                sync(Arc::new(eth_client), Arc::clone(&db), Default::default(), ServiceContext::new_for_testing()).await
            })
        };

        // Set canceled status and fire event
        let _ = contract.setIsCanceled(false).send().await.expect("Should successfully set canceled status to false");
        let _ = contract.fireEvent().send().await.expect("Should successfully fire messaging event");

        // Wait for event processing
        tokio::time::sleep(Duration::from_secs(5)).await;

        let handler_tx = db.get_pending_message_to_l2(0).unwrap().unwrap();

        assert_eq!(handler_tx.tx.nonce, 0);
        assert_eq!(
            handler_tx.tx.contract_address,
            Felt::from_dec_str("3256441166037631918262930812410838598500200462657642943867372734773841898370").unwrap()
        );
        assert_eq!(
            handler_tx.tx.entry_point_selector,
            Felt::from_dec_str("774397379524139446221206168840917193112228400237242521560346153613428128537").unwrap()
        );
        assert_eq!(
            handler_tx.tx.calldata[0],
            Felt::from_dec_str("993696174272377493693496825928908586134624850969").unwrap()
        );

        // Assert the tx hash computed by the worker is correct
        let event_hash = contract
            .getL1ToL2MsgHash()
            .call()
            .await
            .expect("Should successfully get the message hash from the contract")
            ._0
            .to_string();
        assert!(logs_contain(&format!("event hash: {:?}", event_hash)));

        // Assert that the event is well stored in db
        let last_block = db
            .get_l1_messaging_sync_tip()
            .expect("Should successfully retrieve the last synced L1 block with messaging event")
            .unwrap();
        assert_ne!(last_block, 0);
        // TODO: Assert that the transaction has been executed successfully
        assert!(db.get_pending_message_to_l2(0).unwrap().is_some());

        // Explicitly cancel the listen task, else it would be running in the background
        worker_handle.abort();
    }

    /// Test taken from starknet.rs to ensure consistency
    /// https://github.com/xJonathanLEI/starknet-rs/blob/2ddc69479d326ed154df438d22f2d720fbba746e/starknet-core/src/types/msg.rs#L96
    #[rstest]
    #[tokio::test]
    async fn test_msg_to_l2_hash() {
        let TestRunner { db_service: _db, dummy_contract: _contract, eth_client, anvil: _anvil } =
            setup_test_env().await;

        let msg = eth_client
            .calculate_message_hash(&L1HandlerTransactionWithFee {
                paid_fee_on_l1: u128::try_from(Felt::from_bytes_be_slice(U256::ZERO.to_be_bytes_vec().as_slice()))
                    .unwrap(),
                tx: L1HandlerTransaction {
                    version: Felt::ZERO,
                    nonce: 775628,
                    contract_address: Felt::from_hex(
                        "0x73314940630fd6dcda0d772d4c972c4e0a9946bef9dabf4ef84eda8ef542b82",
                    )
                    .expect("Should parse valid destination address hex"),
                    entry_point_selector: Felt::from_hex(
                        "0x2d757788a8d8d6f21d1cd40bce38a8222d70654214e96ff95d8086e684fbee5",
                    )
                    .expect("Should parse valid selector hex"),
                    calldata: vec![
                        Felt::from_bytes_be_slice(
                            Address::from_hex("c3511006C04EF1d78af4C8E0e74Ec18A6E64Ff9e")
                                .unwrap()
                                .0
                                 .0
                                .to_vec()
                                .as_slice(),
                        ),
                        Felt::from_hex("0x689ead7d814e51ed93644bc145f0754839b8dcb340027ce0c30953f38f55d7")
                            .expect("Should parse valid payload[0] hex"),
                        Felt::from_hex("0x2c68af0bb140000").expect("Should parse valid payload[1] hex"),
                        Felt::from_hex("0x0").expect("Should parse valid payload[2] hex"),
                    ]
                    .into(),
                },
            })
            .expect("Should successfully compute L1 to L2 message hash");

        let expected_hash = <[u8; 32]>::from_hex("c51a543ef9563ad2545342b390b67edfcddf9886aa36846cf70382362fc5fab3")
            .expect("Should parse valid expected hash hex");

        assert_eq!(msg, expected_hash);
    }
}

#[cfg(test)]
mod eth_client_event_subscription_test {
    use super::*;
    use crate::eth::{EthereumClient, StarknetCoreContract};
    use crate::gas_price::L1BlockMetrics;
    use crate::state_update::state_update_worker;
    use alloy::{node_bindings::Anvil, providers::ProviderBuilder, sol};
    use mc_db::MadaraBackend;
    use mp_chain_config::ChainConfig;
    use rstest::*;
    use tracing_test::traced_test;
    use url::Url;

    sol!(
        #[sol(rpc, bytecode="60806040526c2387986f739797d912cba45da05f55620a1cae6001556d373446be73aadaaea327cd6fe8886002553480156037575f80fd5b50610228806100455f395ff3fe608060405234801561000f575f80fd5b506004361061004a575f3560e01c806335befa5d1461004e578063382d83e31461006c5780634185df151461008a5780639588eca214610094575b5f80fd5b6100566100b2565b6040516100639190610173565b60405180910390f35b6100746100bb565b60405161008191906101a4565b60405180910390f35b6100926100c4565b005b61009c610153565b6040516100a991906101a4565b60405180910390f35b5f600154905090565b5f600254905090565b5f7f0639349b21e886487cd6b341de2050db8ab202d9c6b0e7a2666d598e5fcf81a690505f620a1caf90505f7f0279b69383ea92624c1ae4378ac7fae6428f47bbd21047ea0290c3653064188590507fd342ddf7a308dec111745b00315c14b7efb2bdae570a6856e088ed0c65a3576c838383604051610146939291906101bd565b60405180910390a1505050565b5f8054905090565b5f819050919050565b61016d8161015b565b82525050565b5f6020820190506101865f830184610164565b92915050565b5f819050919050565b61019e8161018c565b82525050565b5f6020820190506101b75f830184610195565b92915050565b5f6060820190506101d05f830186610195565b6101dd6020830185610164565b6101ea6040830184610195565b94935050505056fea264697066735822122016ed1b830e3661c2614ea337cf14026ade61676af633399ebbaae6397f773d3564736f6c634300081a0033")]
        contract DummyContract {
            uint256 _globalRoot = 2814950447364693428789615812000;
            int256 _blockNumber = 662702;
            uint256 _blockHash = 1119674286844400689540394420005000;

            event LogStateUpdate(uint256 globalRoot, int256 blockNumber, uint256 blockHash);

            function fireEvent() public {
                uint256 globalRoot = 2814950447364693428789615812443623689251959344851195711990387747563915674022;
                int256 blockNumber = 662703;
                uint256 blockHash = 1119674286844400689540394420005977072742999649767515920196535047615668295813;

                emit LogStateUpdate(globalRoot, blockNumber, blockHash);
            }
        }

        function stateBlockNumber() public view returns (int256) {
            return _blockNumber;
        }

        function stateRoot() public view returns (uint256) {
            return _globalRoot;
        }

        function stateBlockHash() public view returns (uint256) {
            return _blockHash;
        }
    );

    const ANOTHER_ANVIL_PORT: u16 = 8548;

    /// Test the event subscription and state update functionality
    ///
    /// This test performs the following steps:
    /// 1. Sets up a mock Ethereum environment using Anvil
    /// 2. Initializes necessary services (Database, Metrics)
    /// 3. Deploys a dummy contract and sets up an Ethereum client
    /// 4. Starts listening for state updates
    /// 5. Fires an event from the dummy contract
    /// 6. Waits for event processing and verifies the block number
    #[traced_test]
    #[rstest]
    #[tokio::test]
    async fn listen_and_update_state_when_event_fired_works() {
        // Start Anvil instance
        let anvil = Anvil::new()
            .block_time(1)
            .chain_id(1337)
            .port(ANOTHER_ANVIL_PORT)
            .try_spawn()
            .expect("failed to spawn anvil instance");

        // Set up chain info
        let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());

        let rpc_url: Url = anvil.endpoint().parse().expect("issue while parsing");
        let provider = ProviderBuilder::new().on_http(rpc_url);

        let contract = DummyContract::deploy(provider.clone()).await.unwrap();
        let core_contract = StarknetCoreContract::new(*contract.address(), provider.clone());

        let eth_client =
            EthereumClient { provider: Arc::new(provider.clone()), l1_core_contract: core_contract.clone() };
        let l1_block_metrics = L1BlockMetrics::register().unwrap();
        let (snd, mut recv) = tokio::sync::watch::channel(None);

        // Start listening for state updates
        let listen_handle = {
            let db = backend.clone();
            tokio::spawn(async move {
                state_update_worker(
                    db,
                    Arc::new(eth_client),
                    ServiceContext::new_for_testing(),
                    snd,
                    Arc::new(l1_block_metrics),
                )
                .await
                .unwrap()
            })
        };

        // Wait for get_initial_state
        recv.changed().await.unwrap();
        assert_eq!(recv.borrow().as_ref().unwrap().block_number, Some(662702));

        let block_in_db = backend.latest_l1_confirmed_block_n();
        assert_eq!(block_in_db, Some(662702), "Block in DB does not match expected L2 block number");

        let _ = contract.fireEvent().send().await.expect("Should successfully fire state update event");

        recv.changed().await.unwrap();
        assert_eq!(recv.borrow().as_ref().unwrap().block_number, Some(662703));

        // Verify the block number
        let block_in_db = backend.latest_l1_confirmed_block_n();

        // Explicitly cancel the listen task, else it would be running in the background
        listen_handle.abort();
        assert_eq!(block_in_db, Some(662703), "Block in DB does not match expected L2 block number");
    }
}
