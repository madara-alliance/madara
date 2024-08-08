use crate::contract::parse_handle_l1_message_transaction;
use alloy::primitives::{keccak256, FixedBytes, U256};
use anyhow::Context;
use blockifier::transaction::transactions::L1HandlerTransaction as BlockifierL1HandlerTransaction;
use dc_db::{messaging_db::LastSyncedEventBlock, DeoxysBackend};
use dc_eth::client::StarknetCoreContract::LogMessageToL2;
use dc_eth::client::{EthereumClient, StarknetCoreContract};
use dp_utils::channel_wait_or_graceful_shutdown;
use futures::StreamExt;
use starknet_api::core::ChainId;
use starknet_api::transaction::{Fee, Transaction, TransactionHash};
use alloy::sol_types::SolValue;
use starknet_api::transaction_hash::get_transaction_hash;
use starknet_types_core::felt::Felt;

pub async fn sync(backend: &DeoxysBackend, client: &EthereumClient, chain_id: &ChainId) -> anyhow::Result<()> {
    log::info!("⟠ Starting L1 Messages Syncing...");
    
    let last_synced_event_block = match backend.messaging_last_synced_l1_block_with_event() {
        Ok(Some(blk)) => blk,
        Ok(None) => {
            unreachable!("Should never be None")
        }
        Err(e) => {
            log::error!("⟠ Madara Messaging DB unavailable: {:?}", e);
            return Err(e.into());
        }
    };
    let event_filter = client.l1_core_contract.event_filter::<StarknetCoreContract::LogMessageToL2>();
    let mut event_stream = event_filter
    .from_block(last_synced_event_block.block_number)
    .watch()
    .await
    .context("Failed to watch event filter")?
    .into_stream();

    while let Some(event_result) = channel_wait_or_graceful_shutdown(event_stream.next()).await {
        if let Ok((event, meta)) = event_result {
            log::info!(
                "⟠ Processing L1 Message from block: {:?}, transaction_hash: {:?}, log_index: {:?}",
                meta.block_number,
                meta.transaction_hash,
                meta.log_index
            );
            

            // Check if cancellation was initiated
            let cancellation_timestamp =
            client.get_l1_to_l2_message_cancellations(get_l1_to_l2_msg_hash(&event)?).await?;
            if cancellation_timestamp != Felt::ZERO {
                log::info!("⟠ L1 Message was cancelled in block at timestamp : {:?}", cancellation_timestamp);
                continue;
            }
            
            match process_l1_message(backend, &event, &meta.block_number, &meta.log_index, chain_id).await {
                Ok(Some(tx_hash)) => {
                    log::info!(
                        "⟠ L1 Message from block: {:?}, transaction_hash: {:?}, log_index: {:?} submitted, \
                        transaction hash on L2: {:?}",
                        meta.block_number,
                        meta.transaction_hash,
                        meta.log_index,
                        tx_hash
                    );
                }
                Ok(None) => {
                }
                Err(e) => {
                    log::error!(
                        "⟠ Unexpected error while processing L1 Message from block: {:?}, transaction_hash: {:?}, \
                    log_index: {:?}, error: {:?}",
                        meta.block_number,
                        meta.transaction_hash,
                        meta.log_index,
                        e
                    )
                }
            }
        }
    }

    Ok(())
}

async fn process_l1_message(
    backend: &DeoxysBackend,
    event: &LogMessageToL2,
    l1_block_number: &Option<u64>,
    event_index: &Option<u64>,
    chain_id: &ChainId,
) -> anyhow::Result<Option<TransactionHash>> {
    let transaction = parse_handle_l1_message_transaction(event)?;
    // Ensure that L1 message has not been executed
    match backend.messaging_update_nonces_if_not_used(transaction.nonce) {
        Ok(true) => {}
        Ok(false) => {
            log::debug!("⟠ Event already processed: {:?}", transaction);
            return Ok(None);
        }
        Err(e) => {
            log::error!("⟠ Unexpected DB error: {:?}", e);
            return Err(e.into());
        }
    };
    let blockifier_transaction: BlockifierL1HandlerTransaction = BlockifierL1HandlerTransaction {
        tx: transaction.clone(),
        tx_hash: get_transaction_hash(&Transaction::L1Handler(transaction.clone()), chain_id, &transaction.version)?,
        paid_fee_on_l1: Fee(event.fee.try_into()?),
    };

    // TODO: submit tx to mempool



    let block_sent = LastSyncedEventBlock::new(
        l1_block_number.unwrap(),
        event_index.unwrap(),
    );
    // TODO: remove unwraps
    backend.messaging_update_last_synced_l1_block_with_event(block_sent)?;

    // TODO: replace by tx hash from mempool 
    Ok(Some(blockifier_transaction.tx_hash))
}

/// Computes the message hashed with the given event data
fn get_l1_to_l2_msg_hash(event: &LogMessageToL2) -> anyhow::Result<FixedBytes<32>> {
    let data = (
        [0u8; 12],
        event.fromAddress.0 .0,
        event.toAddress,
        event.nonce,
        event.selector,
        U256::from(event.payload.len()),
        event.payload.clone(),
    );
    Ok(keccak256(data.abi_encode_packed()))
}

#[cfg(test)]
mod tests {
    use crate::worker::{self, get_l1_to_l2_msg_hash};
    use dc_eth::client::{EthereumClient, L1BlockMetrics, StarknetCoreContract::{self, LogMessageToL2}};

    use std::{sync::Arc, time::Duration};

    use alloy::{hex::FromHex, node_bindings::Anvil, primitives::{Address, U256}, providers::ProviderBuilder, sol};
    use dc_db::DatabaseService;
    use dc_metrics::MetricsService;
    use dp_block::chain_config::ChainConfig;
    use rstest::*;
    use tempfile::TempDir;
    use url::Url;

    // LogMessageToL2 from 0x21980d6674d33e50deee43c6c30ef3b439bd148249b4539ce37b7856ac46b843
    // bytecode is compiled DummyContractBasicTestCase
    sol!(
        #[sol(rpc, bytecode="6080604052348015600e575f80fd5b506104ca8061001c5f395ff3fe608060405234801561000f575f80fd5b5060043610610034575f3560e01c80634185df15146100385780639be446bf14610042575b5f80fd5b610040610072565b005b61005c600480360381019061005791906102eb565b6102ae565b604051610069919061032e565b60405180910390f35b5f73ae0ee0a63a2ce6baeeffe56e7714fb4efe48d41990505f7f073314940630fd6dcda0d772d4c972c4e0a9946bef9dabf4ef84eda8ef542b8290505f7f01b64b1b3b690b43b9b514fb81377518f4039cd3e4f4914d8a6bdf01d679fb1990505f600767ffffffffffffffff8111156100ee576100ed610347565b5b60405190808252806020026020018201604052801561011c5781602001602082028036833780820191505090505b5090506060815f8151811061013457610133610374565b5b602002602001018181525050621950918160018151811061015857610157610374565b5b60200260200101818152505065231594f0c7ea8160028151811061017f5761017e610374565b5b6020026020010181815250506005816003815181106101a1576101a0610374565b5b60200260200101818152505062455448816004815181106101c5576101c4610374565b5b60200260200101818152505073bdb193c166cfb7be2e51711c5648ebeef94063bb816005815181106101fa576101f9610374565b5b6020026020010181815250507e7d79cd86ba27a2508a9ca55c8b3474ca082bc5173d0467824f07a32e9db8888160068151811061023a57610239610374565b5b6020026020010181815250505f662386f26fc1000090505f83858773ffffffffffffffffffffffffffffffffffffffff167fdb80dd488acf86d17c747445b0eabb5d57c541d3bd7b6b87af987858e5066b2b86868660405161029e93929190610458565b60405180910390a4505050505050565b5f919050565b5f80fd5b5f819050919050565b6102ca816102b8565b81146102d4575f80fd5b50565b5f813590506102e5816102c1565b92915050565b5f60208284031215610300576102ff6102b4565b5b5f61030d848285016102d7565b91505092915050565b5f819050919050565b61032881610316565b82525050565b5f6020820190506103415f83018461031f565b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52604160045260245ffd5b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffd5b5f81519050919050565b5f82825260208201905092915050565b5f819050602082019050919050565b6103d381610316565b82525050565b5f6103e483836103ca565b60208301905092915050565b5f602082019050919050565b5f610406826103a1565b61041081856103ab565b935061041b836103bb565b805f5b8381101561044b57815161043288826103d9565b975061043d836103f0565b92505060018101905061041e565b5085935050505092915050565b5f6060820190508181035f83015261047081866103fc565b905061047f602083018561031f565b61048c604083018461031f565b94935050505056fea2646970667358221220d226a3947472b12337ac4b02d3ce43e768d434887dc37254083678a238a5af6c64736f6c634300081a0033")]
        contract DummyContractBasicTestCase {
            event LogMessageToL2(address indexed _fromAddress, uint256 indexed _toAddress, uint256 indexed _selector, uint256[] payload, uint256 nonce, uint256 fee);

            function fireEvent() public {
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
                uint256 nonce= 10000000000000000;
                uint256 fee= 0;

                emit LogMessageToL2(fromAddress, toAddress, selector, payload, nonce,fee);
            }

            function l1ToL2MessageCancellations(bytes32 msgHash) external view returns (uint256) {
                return 0;
            }
        }
    );

    /// Test the event subscription and state update functionality
    ///
    /// This test performs the following steps:
    /// 1. Sets up a mock Ethereum environment using Anvil
    /// 2. Initializes necessary services (Database, Metrics)
    /// 3. Deploys a dummy contract and sets up an Ethereum client
    /// 4. Starts worker
    /// 5. Fires an Message event from the dummy contract
    /// 6. Waits for event processing and verify if the event is well stored
    /// 7. Fire the same event
    /// 8. Waits for event processing and verify that the event is not stored (already processed)
    #[rstest]
    #[tokio::test]
    async fn e2e_test1() {
        // Start Anvil instance
        let anvil = Anvil::new().block_time(1).chain_id(1337).try_spawn().expect("failed to spawn anvil instance");
        println!("Anvil started and running at `{}`", anvil.endpoint());
        
        // Set up chain info
        let chain_info = Arc::new(ChainConfig::test_config());

        // Set up database paths
        let temp_dir = TempDir::new().expect("issue while creating temporary directory");
        let base_path = temp_dir.path().join("data");
        let backup_dir = Some(temp_dir.path().join("backups"));

        // Initialize database service
        let db = Arc::new(
            DatabaseService::new(&base_path, backup_dir, false, chain_info.clone())
                .await
                .expect("Failed to create database service"),
        );

        // Set up metrics service
        let prometheus_service = MetricsService::new(true, false, 9615).unwrap();
        let l1_block_metrics = L1BlockMetrics::register(&prometheus_service.registry()).unwrap();

        let rpc_url: Url = anvil.endpoint().parse().expect("issue while parsing");
        let provider = ProviderBuilder::new().on_http(rpc_url);

        let contract = DummyContractBasicTestCase::deploy(provider.clone()).await.unwrap();
        let core_contract = StarknetCoreContract::new(*contract.address(), provider.clone());

        let eth_client =
            EthereumClient { provider: Arc::new(provider), l1_core_contract: core_contract.clone(), l1_block_metrics };

        // Start worker
        let worker_handle = {
            let db = Arc::clone(&db);
            tokio::spawn(async move {
                worker::sync(
                    db.backend(),
                    &eth_client,
                    &chain_info.chain_id,
                )
                .await
            })
        };

        // Send a Event and wait for processing, Panic if fail
        let _ = contract.fireEvent().send().await.expect("Failed to fire event");
        tokio::time::sleep(Duration::from_secs(5)).await;

        // TODO : Assert that event was caught by the worker with correct data
        // TODO : Assert the tx hash computed by the worker is correct

        // TODO : Assert that the tx has been included in the mempool

        // Assert that the event is well stored in db
        let last_block = db.backend().messaging_last_synced_l1_block_with_event().expect("failed to retrieve block").unwrap();
        assert_ne!(last_block.block_number, 0);

        // Assert that the tx was correctly executed
        
        // Explicitly cancel the listen task, else it would be running in the background
        worker_handle.abort();
    }


    // Basically the same test as first but we send two times the an event with same to be sure is not inserted in db 
    #[rstest]
    #[tokio::test]
    async fn e2e_test2() {
        // Start Anvil instance
        let anvil = Anvil::new().block_time(1).chain_id(1337).try_spawn().expect("failed to spawn anvil instance");
        println!("Anvil started and running at `{}`", anvil.endpoint());
        
        // Set up chain info
        let chain_info = Arc::new(ChainConfig::test_config());

        // Set up database paths
        let temp_dir = TempDir::new().expect("issue while creating temporary directory");
        let base_path = temp_dir.path().join("data");
        let backup_dir = Some(temp_dir.path().join("backups"));

        // Initialize database service
        let db = Arc::new(
            DatabaseService::new(&base_path, backup_dir, false, chain_info.clone())
                .await
                .expect("Failed to create database service"),
        );

        // Set up metrics service
        let prometheus_service = MetricsService::new(true, false, 9615).unwrap();
        let l1_block_metrics = L1BlockMetrics::register(&prometheus_service.registry()).unwrap();

        let rpc_url: Url = anvil.endpoint().parse().expect("issue while parsing");
        let provider = ProviderBuilder::new().on_http(rpc_url);

        let contract = DummyContractBasicTestCase::deploy(provider.clone()).await.unwrap();
        let core_contract = StarknetCoreContract::new(*contract.address(), provider.clone());

        let eth_client =
            EthereumClient { provider: Arc::new(provider), l1_core_contract: core_contract.clone(), l1_block_metrics };

        // Start worker
        let worker_handle = {
            let db = Arc::clone(&db);
            tokio::spawn(async move {
                worker::sync(
                    db.backend(),
                    &eth_client,
                    &chain_info.chain_id,
                )
                .await
            })
        };

        let _ = contract.fireEvent().send().await.expect("Failed to fire event");
        tokio::time::sleep(Duration::from_secs(5)).await;
        let last_block = db.backend().messaging_last_synced_l1_block_with_event().expect("failed to retrieve block").unwrap();
        assert_ne!(last_block.block_number, 0);

        // Send the event a second time
        let _ = contract.fireEvent().send().await.expect("Failed to fire event");
        tokio::time::sleep(Duration::from_secs(5)).await;
        // Assert that the last event in db is still the same as it is already processed (same nonce)
        assert_eq!(last_block.block_number, db.backend().messaging_last_synced_l1_block_with_event().expect("failed to retrieve block").unwrap().block_number);

        worker_handle.abort();
    }

    // LogMessageToL2 from 0x21980d6674d33e50deee43c6c30ef3b439bd148249b4539ce37b7856ac46b843
    // bytecode is compiled DummyContractMessageCancelled
    sol!(
        #[sol(rpc, bytecode="6080604052348015600e575f80fd5b506104d18061001c5f395ff3fe608060405234801561000f575f80fd5b5060043610610034575f3560e01c80634185df15146100385780639be446bf14610042575b5f80fd5b610040610072565b005b61005c600480360381019061005791906102f2565b6102ae565b6040516100699190610335565b60405180910390f35b5f73ae0ee0a63a2ce6baeeffe56e7714fb4efe48d41990505f7f073314940630fd6dcda0d772d4c972c4e0a9946bef9dabf4ef84eda8ef542b8290505f7f01b64b1b3b690b43b9b514fb81377518f4039cd3e4f4914d8a6bdf01d679fb1990505f600767ffffffffffffffff8111156100ee576100ed61034e565b5b60405190808252806020026020018201604052801561011c5781602001602082028036833780820191505090505b5090506060815f815181106101345761013361037b565b5b60200260200101818152505062195091816001815181106101585761015761037b565b5b60200260200101818152505065231594f0c7ea8160028151811061017f5761017e61037b565b5b6020026020010181815250506005816003815181106101a1576101a061037b565b5b60200260200101818152505062455448816004815181106101c5576101c461037b565b5b60200260200101818152505073bdb193c166cfb7be2e51711c5648ebeef94063bb816005815181106101fa576101f961037b565b5b6020026020010181815250507e7d79cd86ba27a2508a9ca55c8b3474ca082bc5173d0467824f07a32e9db8888160068151811061023a5761023961037b565b5b6020026020010181815250505f662386f26fc1000090505f83858773ffffffffffffffffffffffffffffffffffffffff167fdb80dd488acf86d17c747445b0eabb5d57c541d3bd7b6b87af987858e5066b2b86868660405161029e9392919061045f565b60405180910390a4505050505050565b5f6366b4f1059050919050565b5f80fd5b5f819050919050565b6102d1816102bf565b81146102db575f80fd5b50565b5f813590506102ec816102c8565b92915050565b5f60208284031215610307576103066102bb565b5b5f610314848285016102de565b91505092915050565b5f819050919050565b61032f8161031d565b82525050565b5f6020820190506103485f830184610326565b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52604160045260245ffd5b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffd5b5f81519050919050565b5f82825260208201905092915050565b5f819050602082019050919050565b6103da8161031d565b82525050565b5f6103eb83836103d1565b60208301905092915050565b5f602082019050919050565b5f61040d826103a8565b61041781856103b2565b9350610422836103c2565b805f5b8381101561045257815161043988826103e0565b9750610444836103f7565b925050600181019050610425565b5085935050505092915050565b5f6060820190508181035f8301526104778186610403565b90506104866020830185610326565b6104936040830184610326565b94935050505056fea2646970667358221220a00ca1660c5b5790d27d1df904941a54372771b7fd102abf90514c3d2eeff28b64736f6c634300081a0033")]
        contract DummyContractMessageCancelled {
            event LogMessageToL2(address indexed _fromAddress, uint256 indexed _toAddress, uint256 indexed _selector, uint256[] payload, uint256 nonce, uint256 fee);

            function fireEvent() public {
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
                uint256 nonce= 10000000000000000;
                uint256 fee= 0;

                emit LogMessageToL2(fromAddress, toAddress, selector, payload, nonce,fee);
            }

            function l1ToL2MessageCancellations(bytes32 msgHash) external view returns (uint256) {
                return 1723134213;
            }
        }
    );

    #[rstest]
    #[tokio::test]
    async fn e2e_test3() {
        // Start Anvil instance
        let anvil = Anvil::new().block_time(1).chain_id(1337).try_spawn().expect("failed to spawn anvil instance");
        println!("Anvil started and running at `{}`", anvil.endpoint());
        
        // Set up chain info
        let chain_info = Arc::new(ChainConfig::test_config());

        // Set up database paths
        let temp_dir = TempDir::new().expect("issue while creating temporary directory");
        let base_path = temp_dir.path().join("data");
        let backup_dir = Some(temp_dir.path().join("backups"));

        // Initialize database service
        let db = Arc::new(
            DatabaseService::new(&base_path, backup_dir, false, chain_info.clone())
                .await
                .expect("Failed to create database service"),
        );

        // Set up metrics service
        let prometheus_service = MetricsService::new(true, false, 9615).unwrap();
        let l1_block_metrics = L1BlockMetrics::register(&prometheus_service.registry()).unwrap();

        let rpc_url: Url = anvil.endpoint().parse().expect("issue while parsing");
        let provider = ProviderBuilder::new().on_http(rpc_url);

        let contract = DummyContractMessageCancelled::deploy(provider.clone()).await.unwrap();
        let core_contract = StarknetCoreContract::new(*contract.address(), provider.clone());

        let eth_client =
            EthereumClient { provider: Arc::new(provider), l1_core_contract: core_contract.clone(), l1_block_metrics };

        // Start worker
        let worker_handle = {
            let db = Arc::clone(&db);
            tokio::spawn(async move {
                worker::sync(
                    db.backend(),
                    &eth_client,
                    &chain_info.chain_id,
                )
                .await
            })
        };

        let _ = contract.fireEvent().send().await.expect("Failed to fire event");
        tokio::time::sleep(Duration::from_secs(5)).await;
        let last_block = db.backend().messaging_last_synced_l1_block_with_event().expect("failed to retrieve block").unwrap();
        assert_eq!(last_block.block_number, 0);

        worker_handle.abort();
    }

    #[test]
    fn test_msg_to_l2_hash() {
        // Goerli-1 tx: 0374286ae28f201e61ffbc5b022cc9701208640b405ea34ea9799f97d5d2d23c

        let msg = LogMessageToL2 {
            fromAddress: Address::from_hex("0xc3511006C04EF1d78af4C8E0e74Ec18A6E64Ff9e").unwrap(),
            toAddress: U256::from_str_radix("73314940630fd6dcda0d772d4c972c4e0a9946bef9dabf4ef84eda8ef542b82", 16)
                .unwrap(),
            selector: U256::from_str_radix("2d757788a8d8d6f21d1cd40bce38a8222d70654214e96ff95d8086e684fbee5", 16)
                .unwrap(),
            payload: vec![
                U256::from_str_radix("689ead7d814e51ed93644bc145f0754839b8dcb340027ce0c30953f38f55d7", 16).unwrap(),
                U256::from_str_radix("2c68af0bb140000", 16).unwrap(),
                U256::from_str_radix("0", 16).unwrap(),
            ],
            nonce: U256::from_str_radix("775628", 10).unwrap(),
            fee: U256::from_str_radix("0", 10).unwrap(),
        };

        let expected_hash = "0xc51a543ef9563ad2545342b390b67edfcddf9886aa36846cf70382362fc5fab3";

        assert_eq!(get_l1_to_l2_msg_hash(&msg).unwrap().to_string(), expected_hash);
    }
}
