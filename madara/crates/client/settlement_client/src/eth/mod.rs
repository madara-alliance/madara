pub mod error;
pub mod event;

use crate::client::{ClientType, SettlementClientTrait};
use crate::error::SettlementClientError;
use crate::eth::event::EthereumEventStream;
use crate::eth::StarknetCoreContract::{LogMessageToL2, StarknetCoreContractInstance};
use crate::gas_price::L1BlockMetrics;
use crate::messaging::L1toL2MessagingEventData;
use crate::state_update::{update_l1, StateUpdate};
use crate::utils::convert_log_state_update;
use alloy::eips::BlockNumberOrTag;
use alloy::primitives::{keccak256, Address, B256, U256};
use alloy::providers::{Provider, ProviderBuilder, ReqwestProvider, RootProvider};
use alloy::rpc::types::Filter;
use alloy::sol;
use alloy::sol_types::SolValue;
use alloy::transports::http::{Client, Http};
use async_trait::async_trait;
use bitvec::macros::internal::funty::Fundamental;
use error::EthereumClientError;
use futures::StreamExt;
use mc_db::l1_db::LastSyncedEventBlock;
use mc_db::MadaraBackend;
use mp_convert::{felt_to_u256, ToFelt};
use mp_utils::service::ServiceContext;
use starknet_types_core::felt::Felt;
use std::sync::Arc;
use std::time::Duration;
use url::Url;

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
    pub url: Url,
    pub l1_core_address: Address,
}

impl Clone for EthereumClient {
    fn clone(&self) -> Self {
        EthereumClient { provider: Arc::clone(&self.provider), l1_core_contract: self.l1_core_contract.clone() }
    }
}

impl EthereumClient {
    pub async fn new(config: EthereumClientConfig) -> Result<Self, SettlementClientError> {
        let provider = ProviderBuilder::new().on_http(config.url);
        // Check if contract exists
        if !provider
            .get_code_at(config.l1_core_address)
            .await
            .map_err(|e| -> SettlementClientError { EthereumClientError::Rpc(e.to_string()).into() })?
            .is_empty()
        {
            let contract = StarknetCoreContract::new(config.l1_core_address, provider.clone());
            Ok(Self { provider: Arc::new(provider), l1_core_contract: contract })
        } else {
            Err(SettlementClientError::Ethereum(EthereumClientError::Contract(
                "Core contract not found at given address".into(),
            )))
        }
    }
}

const HISTORY_SIZE: usize = 300; // Number of blocks to use for gas price calculation (approx. 1 hour at 12 sec block time)
const POLL_INTERVAL: Duration = Duration::from_secs(5); // Interval between event polling attempts
const EVENT_SEARCH_BLOCK_RANGE: u64 = 6000; // Number of blocks to search backwards for events (approx. 24h at 15 sec block time)

#[async_trait]
impl SettlementClientTrait for EthereumClient {
    type Config = EthereumClientConfig;
    type StreamType = EthereumEventStream;
    fn get_client_type(&self) -> ClientType {
        ClientType::ETH
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

        // Assuming an avg Block time of 15sec we check for a LogStateUpdate occurence in the last ~24h
        let filter = Filter::new()
            .from_block(latest_block.saturating_sub(EVENT_SEARCH_BLOCK_RANGE))
            .to_block(latest_block)
            .address(*self.l1_core_contract.address());

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
            None => {
                let from_block = latest_block.saturating_sub(EVENT_SEARCH_BLOCK_RANGE);
                Err(SettlementClientError::Ethereum(EthereumClientError::EventProcessing {
                    message: format!("no LogStateUpdate event found in block range [{}, {}]", from_block, latest_block),
                    block_number: latest_block,
                }))
            }
        }
    }

    /// Get the last Starknet block number verified on L1
    async fn get_last_verified_block_number(&self) -> Result<u64, SettlementClientError> {
        self.l1_core_contract.stateBlockNumber().call().await.map(|block_number| block_number._0.as_u64()).map_err(
            |e| -> SettlementClientError {
                EthereumClientError::Contract(format!("Failed to get state block number: {}", e)).into()
            },
        )
    }

    /// Get the last Starknet state root verified on L1
    async fn get_last_verified_state_root(&self) -> Result<Felt, SettlementClientError> {
        let state_root = self.l1_core_contract.stateRoot().call().await.map_err(|e| -> SettlementClientError {
            EthereumClientError::Contract(format!("Failed to get state root from L1: {}", e)).into()
        })?;

        Ok(state_root._0.to_felt())
    }

    /// Get the last Starknet block hash verified on L1
    async fn get_last_verified_block_hash(&self) -> Result<Felt, SettlementClientError> {
        let block_hash = self.l1_core_contract.stateBlockHash().call().await.map_err(|e| -> SettlementClientError {
            EthereumClientError::Contract(format!("Failed to get state block hash from L1: {}", e)).into()
        })?;

        Ok(block_hash._0.to_felt())
    }

    async fn get_current_core_contract_state(&self) -> Result<StateUpdate, SettlementClientError> {
        let block_number = self.get_last_verified_block_number().await?;
        let block_hash = self.get_last_verified_block_hash().await?;
        let global_root = self.get_last_verified_state_root().await?;

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
        backend: Arc<MadaraBackend>,
        mut ctx: ServiceContext,
        l1_block_metrics: Arc<L1BlockMetrics>,
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

        // Create a ticker that fires at regular intervals
        let mut interval = tokio::time::interval(POLL_INTERVAL);

        // Process events in a loop until the context is cancelled
        while let Some(Some(event_result)) = ctx
            .run_until_cancelled(async {
                interval.tick().await; // Wait for the next interval tick
                event_stream.next().await
            })
            .await
        {
            let log = event_result.map_err(|e| -> SettlementClientError {
                EthereumClientError::EventStream { message: format!("Failed to process event: {}", e) }.into()
            })?;

            let format_event = convert_log_state_update(log.0.clone()).map_err(|e| -> SettlementClientError {
                EthereumClientError::StateUpdate { message: format!("Failed to convert log state update: {}", e) }
                    .into()
            })?;

            update_l1(&backend, format_event, l1_block_metrics.clone()).map_err(|e| -> SettlementClientError {
                EthereumClientError::StateUpdate { message: format!("Failed to update L1 state: {}", e) }.into()
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

    fn get_messaging_hash(&self, event: &L1toL2MessagingEventData) -> Result<Vec<u8>, SettlementClientError> {
        let payload_vec = event.payload.iter().try_fold(Vec::with_capacity(event.payload.len()), |mut acc, felt| {
            let u256 = felt_to_u256(*felt).map_err(|e| -> SettlementClientError {
                EthereumClientError::Conversion(format!("Failed to convert payload element to U256: {}", e)).into()
            })?;
            acc.push(u256);
            Ok::<_, SettlementClientError>(acc)
        })?;

        let from_address_start_index = event.from.to_bytes_be().as_slice().len().saturating_sub(20);
        // encoding used here is taken from: https://docs.starknet.io/architecture-and-concepts/network-architecture/messaging-mechanism/#l1_l2_message_structure
        let data = (
            [0u8; 12],
            Address::from_slice(&event.from.to_bytes_be().as_slice()[from_address_start_index..]),
            felt_to_u256(event.to).map_err(|e| -> SettlementClientError {
                EthereumClientError::Conversion(format!("Failed to convert 'to' address to U256: {}", e)).into()
            })?,
            felt_to_u256(event.nonce).map_err(|e| -> SettlementClientError {
                EthereumClientError::Conversion(format!("Failed to convert nonce to U256: {}", e)).into()
            })?,
            felt_to_u256(event.selector).map_err(|e| -> SettlementClientError {
                EthereumClientError::Conversion(format!("Failed to convert selector to U256: {}", e)).into()
            })?,
            U256::from(event.payload.len()),
            payload_vec,
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
    /// - A felt representing a timestamp :
    ///     - 0 if the message has not been cancelled
    ///     - timestamp of the cancellation if it has been cancelled
    /// - An Error if the call fail
    async fn get_l1_to_l2_message_cancellations(&self, msg_hash: &[u8]) -> Result<Felt, SettlementClientError> {
        let cancellation_timestamp =
            self.l1_core_contract.l1ToL2MessageCancellations(B256::from_slice(msg_hash)).call().await.map_err(
                |e| -> SettlementClientError {
                    EthereumClientError::L1ToL2Messaging {
                        message: format!("Failed to check message cancellation status: {}", e),
                    }
                    .into()
                },
            )?;

        Ok(cancellation_timestamp._0.to_felt())
    }

    async fn get_messaging_stream(
        &self,
        last_synced_event_block: LastSyncedEventBlock,
    ) -> Result<Self::StreamType, SettlementClientError> {
        let filter = self.l1_core_contract.event_filter::<LogMessageToL2>();
        let event_stream = filter
            .from_block(last_synced_event_block.block_number)
            .to_block(BlockNumberOrTag::Finalized)
            .watch()
            .await
            .map_err(|e| -> SettlementClientError {
                EthereumClientError::ArchiveRequired(format!(
                    "Could not fetch events, archive node may be required: {}",
                    e
                ))
                .into()
            })?;

        Ok(EthereumEventStream::new(event_stream))
    }
}

#[cfg(test)]
pub mod eth_client_getter_test {
    use super::*;
    use alloy::primitives::U256;
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
        let ethereum_client_config = EthereumClientConfig { url: rpc_url, l1_core_address: core_contract_address };
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
    async fn get_last_verified_block_hash_works() {
        let eth_client = create_ethereum_client(get_anvil_url());
        let block_hash =
            eth_client.get_last_verified_block_hash().await.expect("issue while getting the last verified block hash");
        let expected =
            U256::from_str_radix(L2_BLOCK_HASH, 10).expect("Should parse the predefined L2 block hash").to_felt();
        assert_eq!(block_hash, expected, "latest block hash not matching");
    }

    #[tokio::test]
    async fn get_last_state_root_works() {
        let eth_client = create_ethereum_client(get_anvil_url());
        let state_root = eth_client.get_last_verified_state_root().await.expect("issue while getting the state root");
        let expected =
            U256::from_str_radix(L2_STATE_ROOT, 10).expect("Should parse the predefined L2 state root").to_felt();
        assert_eq!(state_root, expected, "latest block state root not matching");
    }

    #[tokio::test]
    async fn get_last_verified_block_number_works() {
        let eth_client = create_ethereum_client(get_anvil_url());
        let block_number = eth_client.get_last_verified_block_number().await.expect("issue");
        assert_eq!(block_number, L2_BLOCK_NUMBER, "verified block number not matching");
    }
}

#[cfg(test)]
mod l1_messaging_tests {

    use self::DummyContract::DummyContractInstance;
    use crate::client::SettlementClientTrait;
    use crate::eth::{EthereumClient, StarknetCoreContract};
    use crate::messaging::{sync, L1toL2MessagingEventData};
    use alloy::{
        hex::FromHex,
        node_bindings::{Anvil, AnvilInstance},
        primitives::{Address, U256},
        providers::{ProviderBuilder, RootProvider},
        sol,
        transports::http::{Client, Http},
    };
    use blockifier::transaction::transaction_execution::Transaction;
    use mc_db::DatabaseService;
    use mc_mempool::MempoolProvider;
    use mc_mempool::{GasPriceProvider, L1DataProvider, Mempool, MempoolLimits};
    use mp_chain_config::ChainConfig;
    use mp_utils::service::ServiceContext;
    use rstest::*;
    use starknet_api::core::{ContractAddress, EntryPointSelector, Nonce};
    use starknet_types_core::felt::Felt;
    use std::{sync::Arc, time::Duration};
    use tracing_test::traced_test;
    use url::Url;

    struct TestRunner {
        #[allow(dead_code)]
        anvil: AnvilInstance, // Not used but needs to stay in scope otherwise it will be dropped
        db_service: Arc<DatabaseService>,
        dummy_contract: DummyContractInstance<Http<Client>, RootProvider<Http<Client>>>,
        eth_client: EthereumClient,
        mempool: Arc<Mempool>,
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
        #[sol(rpc, bytecode="6080604052348015600e575f80fd5b5061081b8061001c5f395ff3fe608060405234801561000f575f80fd5b506004361061004a575f3560e01c80634185df151461004e57806390985ef9146100585780639be446bf14610076578063af56443a146100a6575b5f80fd5b6100566100c2565b005b61006061013b565b60405161006d919061047e565b60405180910390f35b610090600480360381019061008b91906104c5565b6101ac565b60405161009d9190610508565b60405180910390f35b6100c060048036038101906100bb9190610556565b6101d8565b005b5f6100cb6101f3565b905080604001518160200151825f015173ffffffffffffffffffffffffffffffffffffffff167fdb80dd488acf86d17c747445b0eabb5d57c541d3bd7b6b87af987858e5066b2b846060015185608001518660a0015160405161013093929190610638565b60405180910390a450565b5f806101456101f3565b9050805f015173ffffffffffffffffffffffffffffffffffffffff16816020015182608001518360400151846060015151856060015160405160200161019096959493929190610720565b6040516020818303038152906040528051906020012091505090565b5f805f9054906101000a900460ff166101c5575f6101cb565b6366b4f1055b63ffffffff169050919050565b805f806101000a81548160ff02191690831515021790555050565b6101fb61041f565b5f73ae0ee0a63a2ce6baeeffe56e7714fb4efe48d41990505f7f073314940630fd6dcda0d772d4c972c4e0a9946bef9dabf4ef84eda8ef542b8290505f7f01b64b1b3b690b43b9b514fb81377518f4039cd3e4f4914d8a6bdf01d679fb1990505f600767ffffffffffffffff8111156102775761027661078b565b5b6040519080825280602002602001820160405280156102a55781602001602082028036833780820191505090505b5090506060815f815181106102bd576102bc6107b8565b5b60200260200101818152505062195091816001815181106102e1576102e06107b8565b5b60200260200101818152505065231594f0c7ea81600281518110610308576103076107b8565b5b60200260200101818152505060058160038151811061032a576103296107b8565b5b602002602001018181525050624554488160048151811061034e5761034d6107b8565b5b60200260200101818152505073bdb193c166cfb7be2e51711c5648ebeef94063bb81600581518110610383576103826107b8565b5b6020026020010181815250507e7d79cd86ba27a2508a9ca55c8b3474ca082bc5173d0467824f07a32e9db888816006815181106103c3576103c26107b8565b5b6020026020010181815250505f806040518060c001604052808773ffffffffffffffffffffffffffffffffffffffff16815260200186815260200185815260200184815260200183815260200182815250965050505050505090565b6040518060c001604052805f73ffffffffffffffffffffffffffffffffffffffff1681526020015f81526020015f8152602001606081526020015f81526020015f81525090565b5f819050919050565b61047881610466565b82525050565b5f6020820190506104915f83018461046f565b92915050565b5f80fd5b6104a481610466565b81146104ae575f80fd5b50565b5f813590506104bf8161049b565b92915050565b5f602082840312156104da576104d9610497565b5b5f6104e7848285016104b1565b91505092915050565b5f819050919050565b610502816104f0565b82525050565b5f60208201905061051b5f8301846104f9565b92915050565b5f8115159050919050565b61053581610521565b811461053f575f80fd5b50565b5f813590506105508161052c565b92915050565b5f6020828403121561056b5761056a610497565b5b5f61057884828501610542565b91505092915050565b5f81519050919050565b5f82825260208201905092915050565b5f819050602082019050919050565b6105b3816104f0565b82525050565b5f6105c483836105aa565b60208301905092915050565b5f602082019050919050565b5f6105e682610581565b6105f0818561058b565b93506105fb8361059b565b805f5b8381101561062b57815161061288826105b9565b975061061d836105d0565b9250506001810190506105fe565b5085935050505092915050565b5f6060820190508181035f83015261065081866105dc565b905061065f60208301856104f9565b61066c60408301846104f9565b949350505050565b5f819050919050565b61068e610689826104f0565b610674565b82525050565b5f81905092915050565b6106a7816104f0565b82525050565b5f6106b8838361069e565b60208301905092915050565b5f6106ce82610581565b6106d88185610694565b93506106e38361059b565b805f5b838110156107135781516106fa88826106ad565b9750610705836105d0565b9250506001810190506106e6565b5085935050505092915050565b5f61072b828961067d565b60208201915061073b828861067d565b60208201915061074b828761067d565b60208201915061075b828661067d565b60208201915061076b828561067d565b60208201915061077b82846106c4565b9150819050979650505050505050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52604160045260245ffd5b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffdfea264697066735822122086bad896bcbfb2835e470442a0eea534e7731e4e85f0b2bd724e41a094608a8264736f6c634300081a0033")]
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
    /// 7. Assert that the tx is succesfully submited to the mempool
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
        let db = Arc::new(DatabaseService::open_for_testing(chain_config.clone()));

        let l1_gas_setter = GasPriceProvider::new();
        let l1_data_provider: Arc<dyn L1DataProvider> = Arc::new(l1_gas_setter.clone());

        let mempool = Arc::new(Mempool::new(
            Arc::clone(db.backend()),
            Arc::clone(&l1_data_provider),
            MempoolLimits::for_testing(),
        ));

        // Set up provider
        let rpc_url: Url = anvil.endpoint().parse().expect("issue while parsing");
        let provider = ProviderBuilder::new().on_http(rpc_url);

        // Set up dummy contract
        let contract = DummyContract::deploy(provider.clone()).await.unwrap();

        let core_contract = StarknetCoreContract::new(*contract.address(), provider.clone());

        let eth_client =
            EthereumClient { provider: Arc::new(provider.clone()), l1_core_contract: core_contract.clone() };

        TestRunner { anvil, db_service: db, dummy_contract: contract, eth_client, mempool }
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
    /// 7. TODO : Assert that the tx is succesfully submited to the mempool
    /// 8. Assert that the event is successfully pushed to the db
    /// 9. TODO : Assert that the tx was correctly executed
    #[rstest]
    #[traced_test]
    #[tokio::test]
    async fn e2e_test_basic_workflow(#[future] setup_test_env: TestRunner) {
        let TestRunner { db_service: db, dummy_contract: contract, eth_client, anvil: _anvil, mempool } =
            setup_test_env.await;

        // Start worker handle
        let worker_handle = {
            let db = Arc::clone(&db);
            let mempool = Arc::clone(&mempool);
            tokio::spawn(async move {
                sync(Arc::new(eth_client), Arc::clone(db.backend()), mempool, ServiceContext::new_for_testing()).await
            })
        };

        // Set canceled status and fire event
        let _ = contract.setIsCanceled(false).send().await.expect("Should successfully set canceled status to false");
        let _ = contract.fireEvent().send().await.expect("Should successfully fire messaging event");

        // Wait for event processing
        tokio::time::sleep(Duration::from_secs(5)).await;

        let expected_nonce = Nonce(Felt::from_dec_str("0").expect("failed to parse nonce string"));

        let current_nonce = mempool.backend.get_l1_messaging_nonce_latest().unwrap().unwrap();
        assert_eq!(current_nonce, expected_nonce);

        // Check that the L1 message correctly trigger an L1 handler tx, which is accepted in Mempool
        // As it has the first nonce (and no other L1 message was received before) we can take it from Mempool
        // TODO: we can add a test case on which a message with Nonce = 1 is being received before this one
        // and check we cant take it until the first on is processed
        let (handler_tx, _handler_tx_hash) = match mempool.tx_take().unwrap().tx {
            Transaction::L1HandlerTransaction(handler_tx) => (handler_tx.tx, handler_tx.tx_hash.0),
            Transaction::AccountTransaction(_) => panic!("Expecting L1 handler transaction"),
        };

        assert_eq!(handler_tx.nonce, expected_nonce);
        assert_eq!(
            handler_tx.contract_address,
            ContractAddress::try_from(
                Felt::from_dec_str("3256441166037631918262930812410838598500200462657642943867372734773841898370")
                    .unwrap()
            )
            .unwrap()
        );
        assert_eq!(
            handler_tx.entry_point_selector,
            EntryPointSelector(
                Felt::from_dec_str("774397379524139446221206168840917193112228400237242521560346153613428128537")
                    .unwrap()
            )
        );
        assert_eq!(
            handler_tx.calldata.0[0],
            Felt::from_dec_str("993696174272377493693496825928908586134624850969").unwrap()
        );

        // Assert that event was caught by the worker with correct data
        // TODO: Maybe add some more assert
        assert!(logs_contain("fromAddress: \"0xae0ee0a63a2ce6baeeffe56e7714fb4efe48d419\""));

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
            .backend()
            .messaging_last_synced_l1_block_with_event()
            .expect("Should successfully retrieve the last synced L1 block with messaging event")
            .unwrap();
        assert_ne!(last_block.block_number, 0);
        // TODO: Assert that the transaction has been executed successfully
        assert!(db.backend().has_l1_messaging_nonce(expected_nonce).unwrap());

        // Explicitly cancel the listen task, else it would be running in the background
        worker_handle.abort();
    }

    /// Test the workflow of l1 -> l2 messaging with duplicate event
    ///
    /// This test performs the following steps:
    /// 1. Sets up test environemment
    /// 2. Starts worker
    /// 3. Fires a Message event from the dummy contract
    /// 4. Waits for event to be processed
    /// 5. Assert that the event is well stored in db
    /// 6. Fires a Message with the same event from the dummy contract
    /// 7. Assert that the last event stored is the first one
    #[rstest]
    #[traced_test]
    #[tokio::test]
    async fn e2e_test_already_processed_event(#[future] setup_test_env: TestRunner) {
        let TestRunner { db_service: db, dummy_contract: contract, eth_client, anvil: _anvil, mempool } =
            setup_test_env.await;

        // Start worker handle
        let worker_handle = {
            let db = Arc::clone(&db);
            tokio::spawn(async move {
                sync(Arc::new(eth_client), Arc::clone(db.backend()), mempool, ServiceContext::new_for_testing()).await
            })
        };

        // Set canceled status and fire first event
        let _ = contract.setIsCanceled(false).send().await.expect("Should successfully set canceled status to false");
        let _ = contract.fireEvent().send().await.expect("Should successfully fire first messaging event");

        // Wait for event processing
        tokio::time::sleep(Duration::from_secs(5)).await;
        let last_block = db
            .backend()
            .messaging_last_synced_l1_block_with_event()
            .expect("Should successfully retrieve the last synced block after first event")
            .unwrap();
        assert_ne!(last_block.block_number, 0);
        let expected_nonce = Nonce(Felt::from_dec_str("0").expect("failed to parse nonce string"));
        assert!(db.backend().has_l1_messaging_nonce(expected_nonce).unwrap());

        // Fire second event
        let _ = contract.fireEvent().send().await.expect("Should successfully fire second messaging event");

        // Wait for event processing
        tokio::time::sleep(Duration::from_secs(5)).await;
        // Assert that the last event in db is still the same as it is already processed (same nonce)
        assert_eq!(
            last_block.block_number,
            db.backend()
                .messaging_last_synced_l1_block_with_event()
                .expect("Should successfully retrieve the last synced block after second event")
                .unwrap()
                .block_number
        );
        assert!(logs_contain("Event already processed"));

        worker_handle.abort();
    }

    /// Test the workflow of l1 -> l2 messaging with message cancelled
    ///
    /// This test performs the following steps:
    /// 1. Sets up test environemment
    /// 2. Starts worker
    /// 3. Fires a Message event from the dummy contract
    /// 4. Waits for event to be processed
    /// 5. Assert that the event is not stored in db
    #[rstest]
    #[traced_test]
    #[tokio::test]
    async fn e2e_test_message_canceled(#[future] setup_test_env: TestRunner) {
        let TestRunner { db_service: db, dummy_contract: contract, eth_client, anvil: _anvil, mempool } =
            setup_test_env.await;

        // Start worker handle
        let worker_handle = {
            let db = Arc::clone(&db);
            tokio::spawn(async move {
                sync(Arc::new(eth_client), Arc::clone(db.backend()), mempool, ServiceContext::new_for_testing()).await
            })
        };

        // Mock cancelled message
        let _ = contract.setIsCanceled(true).send().await.expect("Should successfully set canceled status to true");
        let _ = contract.fireEvent().send().await.expect("Should successfully fire messaging event");

        // Wait for event processing
        tokio::time::sleep(Duration::from_secs(5)).await;
        let last_block = db
            .backend()
            .messaging_last_synced_l1_block_with_event()
            .expect("Should successfully retrieve the last synced block after canceled event")
            .unwrap();
        assert_eq!(last_block.block_number, 0);
        let nonce = Nonce(Felt::from_dec_str("0").expect("Should parse the known valid test nonce"));
        // cancelled message nonce should be inserted to avoid reprocessing
        assert!(db.backend().has_l1_messaging_nonce(nonce).unwrap());
        assert!(logs_contain("Message was cancelled in block at timestamp: 0x66b4f105"));

        worker_handle.abort();
    }

    /// Test taken from starknet.rs to ensure consistency
    /// https://github.com/xJonathanLEI/starknet-rs/blob/2ddc69479d326ed154df438d22f2d720fbba746e/starknet-core/src/types/msg.rs#L96
    #[rstest]
    #[tokio::test]
    async fn test_msg_to_l2_hash() {
        let TestRunner { db_service: _db, dummy_contract: _contract, eth_client, anvil: _anvil, mempool: _mempool } =
            setup_test_env().await;

        let msg = eth_client
            .get_messaging_hash(&L1toL2MessagingEventData {
                from: Felt::from_bytes_be_slice(
                    Address::from_hex("c3511006C04EF1d78af4C8E0e74Ec18A6E64Ff9e").unwrap().0 .0.to_vec().as_slice(),
                ),
                to: Felt::from_hex("0x73314940630fd6dcda0d772d4c972c4e0a9946bef9dabf4ef84eda8ef542b82")
                    .expect("Should parse valid destination address hex"),
                selector: Felt::from_hex("0x2d757788a8d8d6f21d1cd40bce38a8222d70654214e96ff95d8086e684fbee5")
                    .expect("Should parse valid selector hex"),
                payload: vec![
                    Felt::from_hex("0x689ead7d814e51ed93644bc145f0754839b8dcb340027ce0c30953f38f55d7")
                        .expect("Should parse valid payload[0] hex"),
                    Felt::from_hex("0x2c68af0bb140000").expect("Should parse valid payload[1] hex"),
                    Felt::from_hex("0x0").expect("Should parse valid payload[2] hex"),
                ],
                nonce: Felt::from_bytes_be_slice(U256::from(775628).to_be_bytes_vec().as_slice()),
                fee: Some(u128::try_from(Felt::from_bytes_be_slice(U256::ZERO.to_be_bytes_vec().as_slice())).unwrap()),
                transaction_hash: Felt::ZERO,
                message_hash: None,
                block_number: 0,
                event_index: None,
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
    use std::{sync::Arc, time::Duration};

    use crate::eth::event::EthereumEventStream;
    use crate::eth::{EthereumClient, EthereumClientConfig, StarknetCoreContract};
    use crate::state_update::state_update_worker;
    use alloy::{node_bindings::Anvil, providers::ProviderBuilder, sol};
    use mc_db::DatabaseService;
    use mp_chain_config::ChainConfig;
    use rstest::*;
    use url::Url;

    sol!(
        #[sol(rpc, bytecode="6080604052348015600e575f80fd5b506101618061001c5f395ff3fe608060405234801561000f575f80fd5b5060043610610029575f3560e01c80634185df151461002d575b5f80fd5b610035610037565b005b5f7f0639349b21e886487cd6b341de2050db8ab202d9c6b0e7a2666d598e5fcf81a690505f620a1caf90505f7f0279b69383ea92624c1ae4378ac7fae6428f47bbd21047ea0290c3653064188590507fd342ddf7a308dec111745b00315c14b7efb2bdae570a6856e088ed0c65a3576c8383836040516100b9939291906100f6565b60405180910390a1505050565b5f819050919050565b6100d8816100c6565b82525050565b5f819050919050565b6100f0816100de565b82525050565b5f6060820190506101095f8301866100cf565b61011660208301856100e7565b61012360408301846100cf565b94935050505056fea2646970667358221220fbc6fd165c86ed9af0c5fcab2830d4a72894fd6a98e9c16dbf9101c4c22e2f7d64736f6c634300081a0033")]
        contract DummyContract {
            event LogStateUpdate(uint256 globalRoot, int256 blockNumber, uint256 blockHash);

            function fireEvent() public {
                uint256 globalRoot = 2814950447364693428789615812443623689251959344851195711990387747563915674022;
                int256 blockNumber = 662703;
                uint256 blockHash = 1119674286844400689540394420005977072742999649767515920196535047615668295813;

                emit LogStateUpdate(globalRoot, blockNumber, blockHash);
            }
        }
    );

    const L2_BLOCK_NUMBER: u64 = 662703;
    const ANOTHER_ANVIL_PORT: u16 = 8548;
    const EVENT_PROCESSING_TIME: u64 = 2; // Time to allow for event processing in seconds

    /// Test the event subscription and state update functionality
    ///
    /// This test performs the following steps:
    /// 1. Sets up a mock Ethereum environment using Anvil
    /// 2. Initializes necessary services (Database, Metrics)
    /// 3. Deploys a dummy contract and sets up an Ethereum client
    /// 4. Starts listening for state updates
    /// 5. Fires an event from the dummy contract
    /// 6. Waits for event processing and verifies the block number
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
        let chain_config = Arc::new(ChainConfig::madara_test());

        // Initialize database service
        let db = Arc::new(DatabaseService::open_for_testing(chain_config.clone()));

        let rpc_url: Url = anvil.endpoint().parse().expect("issue while parsing");
        let provider = ProviderBuilder::new().on_http(rpc_url);

        let contract = DummyContract::deploy(provider.clone()).await.unwrap();
        let core_contract = StarknetCoreContract::new(*contract.address(), provider.clone());

        let eth_client =
            EthereumClient { provider: Arc::new(provider.clone()), l1_core_contract: core_contract.clone() };
        let l1_block_metrics = L1BlockMetrics::register().unwrap();

        // Start listening for state updates
        let listen_handle = {
            let db = Arc::clone(&db);
            tokio::spawn(async move {
                state_update_worker::<EthereumClientConfig, EthereumEventStream>(
                    Arc::clone(db.backend()),
                    Arc::new(eth_client),
                    ServiceContext::new_for_testing(),
                    Arc::new(l1_block_metrics),
                )
                .await
                .unwrap()
            })
        };

        let _ = contract.fireEvent().send().await.expect("Should successfully fire state update event");

        // Wait for event processing
        tokio::time::sleep(Duration::from_secs(EVENT_PROCESSING_TIME)).await;

        // Verify the block number
        let block_in_db = db
            .backend()
            .get_l1_last_confirmed_block()
            .expect("Should successfully retrieve the last confirmed block number from the database");

        // Explicitly cancel the listen task, else it would be running in the background
        listen_handle.abort();
        assert_eq!(block_in_db, Some(L2_BLOCK_NUMBER), "Block in DB does not match expected L2 block number");
    }
}
