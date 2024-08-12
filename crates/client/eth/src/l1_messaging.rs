use anyhow::Context;
use futures::StreamExt;
use std::sync::Arc;

use crate::client::StarknetCoreContract::LogMessageToL2;
use crate::client::{EthereumClient, StarknetCoreContract};
use crate::utils::u256_to_felt;
use alloy::primitives::{keccak256, FixedBytes, U256};
use alloy::sol_types::SolValue;
use blockifier::transaction::transactions::L1HandlerTransaction as BlockifierL1HandlerTransaction;
use dc_db::{messaging_db::LastSyncedEventBlock, DeoxysBackend};
use dp_utils::channel_wait_or_graceful_shutdown;
use starknet_api::core::{ChainId, ContractAddress, EntryPointSelector, Nonce};
use starknet_api::transaction::{
    Calldata, Fee, L1HandlerTransaction, Transaction, TransactionHash, TransactionVersion,
};
use starknet_api::transaction_hash::get_transaction_hash;
use starknet_types_core::felt::Felt;

pub async fn sync(backend: &DeoxysBackend, client: &EthereumClient, chain_id: &ChainId) -> anyhow::Result<()> {
    tracing::info!("⟠ Starting L1 Messages Syncing...");

    let last_synced_event_block = match backend.messaging_last_synced_l1_block_with_event() {
        Ok(Some(blk)) => blk,
        Ok(None) => {
            unreachable!("Should never be None")
        }
        Err(e) => {
            tracing::error!("⟠ Madara Messaging DB unavailable: {:?}", e);
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
            tracing::info!(
                "⟠ Processing L1 Message from block: {:?}, transaction_hash: {:?}, log_index: {:?}, fromAddress: {:?}",
                meta.block_number,
                meta.transaction_hash,
                meta.log_index,
                event.fromAddress
            );

            // Check if cancellation was initiated
            let event_hash = get_l1_to_l2_msg_hash(&event)?;
            tracing::info!("⟠ Checking for cancelation, event hash : {:?}", event_hash);
            let cancellation_timestamp = client.get_l1_to_l2_message_cancellations(event_hash).await?;
            if cancellation_timestamp != Felt::ZERO {
                tracing::info!("⟠ L1 Message was cancelled in block at timestamp : {:?}", cancellation_timestamp);
                continue;
            }

            match process_l1_message(backend, &event, &meta.block_number, &meta.log_index, chain_id).await {
                Ok(Some(tx_hash)) => {
                    tracing::info!(
                        "⟠ L1 Message from block: {:?}, transaction_hash: {:?}, log_index: {:?} submitted, \
                        transaction hash on L2: {:?}",
                        meta.block_number,
                        meta.transaction_hash,
                        meta.log_index,
                        tx_hash
                    );
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::error!(
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
            tracing::debug!("⟠ Event already processed: {:?}", transaction);
            return Ok(None);
        }
        Err(e) => {
            tracing::error!("⟠ Unexpected DB error: {:?}", e);
            return Err(e.into());
        }
    };
    let blockifier_transaction: BlockifierL1HandlerTransaction = BlockifierL1HandlerTransaction {
        tx: transaction.clone(),
        tx_hash: get_transaction_hash(&Transaction::L1Handler(transaction.clone()), chain_id, &transaction.version)?,
        paid_fee_on_l1: Fee(event.fee.try_into()?),
    };

    // TODO: submit tx to mempool

    let block_sent = LastSyncedEventBlock::new(l1_block_number.unwrap(), event_index.unwrap());
    // TODO: remove unwraps
    backend.messaging_update_last_synced_l1_block_with_event(block_sent)?;

    // TODO: replace by tx hash from mempool
    Ok(Some(blockifier_transaction.tx_hash))
}

pub fn parse_handle_l1_message_transaction(event: &LogMessageToL2) -> anyhow::Result<L1HandlerTransaction> {
    // L1 from address.
    let from_address = u256_to_felt(event.fromAddress.into_word().into())?;

    // L2 contract to call.
    let contract_address = u256_to_felt(event.toAddress)?;

    // Function of the contract to call.
    let entry_point_selector = u256_to_felt(event.selector)?;

    // L1 message nonce.
    let nonce = u256_to_felt(event.nonce)?;

    let event_payload =
        event.payload.clone().into_iter().map(|param| u256_to_felt(param)).collect::<anyhow::Result<Vec<_>>>()?;

    let calldata: Calldata = {
        let mut calldata: Vec<_> = Vec::with_capacity(event.payload.len() + 1);
        calldata.push(from_address);
        calldata.extend(event_payload);

        Calldata(Arc::new(calldata))
    };

    Ok(L1HandlerTransaction {
        nonce: Nonce(nonce),
        contract_address: ContractAddress(contract_address.try_into()?),
        entry_point_selector: EntryPointSelector(entry_point_selector),
        calldata,
        version: TransactionVersion(Felt::ZERO),
    })
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

    use std::{sync::Arc, time::Duration};

    use super::Felt;
    use crate::{
        client::{
            EthereumClient, L1BlockMetrics,
            StarknetCoreContract::{self, LogMessageToL2},
        },
        l1_messaging::get_l1_to_l2_msg_hash,
        utils::felt_to_u256,
    };
    use alloy::{
        hex::FromHex,
        node_bindings::{Anvil, AnvilInstance},
        primitives::{Address, U256},
        providers::{ProviderBuilder, RootProvider},
        sol,
        transports::http::{Client, Http},
    };
    use dc_db::DatabaseService;
    use dc_metrics::MetricsService;
    use dp_block::chain_config::ChainConfig;
    use rstest::*;
    use tempfile::TempDir;
    use tracing_test::traced_test;
    use url::Url;

    use crate::l1_messaging::sync;

    use self::DummyContract::DummyContractInstance;

    struct TestRunner {
        #[allow(dead_code)]
        anvil: AnvilInstance,
        chain_config: Arc<ChainConfig>,
        db_service: Arc<DatabaseService>,
        dummy_contract: DummyContractInstance<Http<Client>, RootProvider<Http<Client>>>,
        eth_client: EthereumClient,
    }

    // LogMessageToL2 from https://etherscan.io/tx/0x21980d6674d33e50deee43c6c30ef3b439bd148249b4539ce37b7856ac46b843
    // bytecode is compiled DummyContractBasicTestCase
    sol!(
        #[derive(Debug)]
        #[sol(rpc, bytecode="6080604052348015600e575f80fd5b5061092c8061001c5f395ff3fe608060405234801561000f575f80fd5b506004361061004a575f3560e01c80634185df151461004e57806390985ef9146100585780639be446bf14610076578063af56443a146100a6575b5f80fd5b6100566100c2565b005b6100606102fe565b60405161006d919061058f565b60405180910390f35b610090600480360381019061008b91906105d6565b610530565b60405161009d9190610619565b60405180910390f35b6100c060048036038101906100bb9190610667565b61055c565b005b5f73ae0ee0a63a2ce6baeeffe56e7714fb4efe48d41990505f7f073314940630fd6dcda0d772d4c972c4e0a9946bef9dabf4ef84eda8ef542b8290505f7f01b64b1b3b690b43b9b514fb81377518f4039cd3e4f4914d8a6bdf01d679fb1990505f600767ffffffffffffffff81111561013e5761013d610692565b5b60405190808252806020026020018201604052801561016c5781602001602082028036833780820191505090505b5090506060815f81518110610184576101836106bf565b5b60200260200101818152505062195091816001815181106101a8576101a76106bf565b5b60200260200101818152505065231594f0c7ea816002815181106101cf576101ce6106bf565b5b6020026020010181815250506005816003815181106101f1576101f06106bf565b5b6020026020010181815250506245544881600481518110610215576102146106bf565b5b60200260200101818152505073bdb193c166cfb7be2e51711c5648ebeef94063bb8160058151811061024a576102496106bf565b5b6020026020010181815250507e7d79cd86ba27a2508a9ca55c8b3474ca082bc5173d0467824f07a32e9db8888160068151811061028a576102896106bf565b5b6020026020010181815250505f662386f26fc1000090505f83858773ffffffffffffffffffffffffffffffffffffffff167fdb80dd488acf86d17c747445b0eabb5d57c541d3bd7b6b87af987858e5066b2b8686866040516102ee939291906107a3565b60405180910390a4505050505050565b5f8073ae0ee0a63a2ce6baeeffe56e7714fb4efe48d41990505f7f073314940630fd6dcda0d772d4c972c4e0a9946bef9dabf4ef84eda8ef542b8290505f7f01b64b1b3b690b43b9b514fb81377518f4039cd3e4f4914d8a6bdf01d679fb1990505f600767ffffffffffffffff81111561037b5761037a610692565b5b6040519080825280602002602001820160405280156103a95781602001602082028036833780820191505090505b5090506060815f815181106103c1576103c06106bf565b5b60200260200101818152505062195091816001815181106103e5576103e46106bf565b5b60200260200101818152505065231594f0c7ea8160028151811061040c5761040b6106bf565b5b60200260200101818152505060058160038151811061042e5761042d6106bf565b5b6020026020010181815250506245544881600481518110610452576104516106bf565b5b60200260200101818152505073bdb193c166cfb7be2e51711c5648ebeef94063bb81600581518110610487576104866106bf565b5b6020026020010181815250507e7d79cd86ba27a2508a9ca55c8b3474ca082bc5173d0467824f07a32e9db888816006815181106104c7576104c66106bf565b5b6020026020010181815250505f662386f26fc1000090508473ffffffffffffffffffffffffffffffffffffffff168482858551866040516020016105109695949392919061088b565b604051602081830303815290604052805190602001209550505050505090565b5f805f9054906101000a900460ff16610549575f61054f565b6366b4f1055b63ffffffff169050919050565b805f806101000a81548160ff02191690831515021790555050565b5f819050919050565b61058981610577565b82525050565b5f6020820190506105a25f830184610580565b92915050565b5f80fd5b6105b581610577565b81146105bf575f80fd5b50565b5f813590506105d0816105ac565b92915050565b5f602082840312156105eb576105ea6105a8565b5b5f6105f8848285016105c2565b91505092915050565b5f819050919050565b61061381610601565b82525050565b5f60208201905061062c5f83018461060a565b92915050565b5f8115159050919050565b61064681610632565b8114610650575f80fd5b50565b5f813590506106618161063d565b92915050565b5f6020828403121561067c5761067b6105a8565b5b5f61068984828501610653565b91505092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52604160045260245ffd5b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffd5b5f81519050919050565b5f82825260208201905092915050565b5f819050602082019050919050565b61071e81610601565b82525050565b5f61072f8383610715565b60208301905092915050565b5f602082019050919050565b5f610751826106ec565b61075b81856106f6565b935061076683610706565b805f5b8381101561079657815161077d8882610724565b97506107888361073b565b925050600181019050610769565b5085935050505092915050565b5f6060820190508181035f8301526107bb8186610747565b90506107ca602083018561060a565b6107d7604083018461060a565b949350505050565b5f819050919050565b6107f96107f482610601565b6107df565b82525050565b5f81905092915050565b61081281610601565b82525050565b5f6108238383610809565b60208301905092915050565b5f610839826106ec565b61084381856107ff565b935061084e83610706565b805f5b8381101561087e5781516108658882610818565b97506108708361073b565b925050600181019050610851565b5085935050505092915050565b5f61089682896107e8565b6020820191506108a682886107e8565b6020820191506108b682876107e8565b6020820191506108c682866107e8565b6020820191506108d682856107e8565b6020820191506108e6828461082f565b915081905097965050505050505056fea2646970667358221220c79bed64ab18032d1c86cfe1e6c7a5424bd9f3aa0e5d5508f706614e45a6ade264736f6c634300081a0033")]
        contract DummyContract {
            bool isCanceled;
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
                return isCanceled ? 1723134213 : 0;
            }

            function setIsCanceled(bool value) public {
                isCanceled = value;
            }

            function getL1ToL2MsgHash() external pure returns (bytes32) {
                // decimal for 0xae0ee0a63a2ce6baeeffe56e7714fb4efe48d419
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

                return keccak256(
                    abi.encodePacked(
                        uint256(uint160(fromAddress)),
                        toAddress,
                        nonce,
                        selector,
                        payload.length,
                        payload
                ));
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
    /// 7. TODO : Assert that the tx is succesfully submited to the mempool
    /// 8. Assert that the event is successfully pushed to the db
    /// 9. TODO : Assert that the tx was correctly executed
    #[fixture]
    async fn setup_test_env() -> TestRunner {
        // Start Anvil instance
        let anvil = Anvil::new().block_time(1).chain_id(1337).try_spawn().expect("failed to spawn anvil instance");
        println!("Anvil started and running at `{}`", anvil.endpoint());

        // Set up chain info
        let chain_config = Arc::new(ChainConfig::test_config());

        // Set up database paths
        let temp_dir = TempDir::new().expect("issue while creating temporary directory");
        let base_path = temp_dir.path().join("data");
        let backup_dir = Some(temp_dir.path().join("backups"));

        // Initialize database service
        let db = Arc::new(
            DatabaseService::new(&base_path, backup_dir, false, chain_config.clone())
                .await
                .expect("Failed to create database service"),
        );

        // Set up metrics service
        let prometheus_service = MetricsService::new(true, false, 9615).unwrap();
        let l1_block_metrics = L1BlockMetrics::register(&prometheus_service.registry()).unwrap();

        // Set up provider
        let rpc_url: Url = anvil.endpoint().parse().expect("issue while parsing");
        let provider = ProviderBuilder::new().on_http(rpc_url);

        // Set up dummy contract
        let contract = DummyContract::deploy(provider.clone()).await.unwrap();

        let core_contract = StarknetCoreContract::new(*contract.address(), provider.clone());

        let eth_client = EthereumClient {
            provider: Arc::new(provider.clone()),
            l1_core_contract: core_contract.clone(),
            l1_block_metrics: l1_block_metrics.clone(),
        };

        TestRunner { anvil, chain_config, db_service: db, dummy_contract: contract, eth_client }
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
        let TestRunner { chain_config, db_service: db, dummy_contract: contract, eth_client, anvil: _anvil } =
            setup_test_env.await;

        // Start worker
        let worker_handle = {
            let db = Arc::clone(&db);
            tokio::spawn(async move { sync(db.backend(), &eth_client, &chain_config.chain_id).await })
        };

        let _ = contract.setIsCanceled(false).send().await;
        // Send a Event and wait for processing, Panic if fail
        let _ = contract.fireEvent().send().await.expect("Failed to fire event");
        tokio::time::sleep(Duration::from_secs(5)).await;

        // Assert that event was caught by the worker with correct data
        // TODO: Maybe add some more assert
        assert!(logs_contain("fromAddress: 0xae0ee0a63a2ce6baeeffe56e7714fb4efe48d419"));

        // Assert the tx hash computed by the worker is correct
        assert!(logs_contain(
            format!("event hash : {:?}", contract.getL1ToL2MsgHash().call().await.expect("failed to get hash")._0)
                .as_str()
        ));

        // TODO : Assert that the tx has been included in the mempool

        // Assert that the event is well stored in db
        let last_block =
            db.backend().messaging_last_synced_l1_block_with_event().expect("failed to retrieve block").unwrap();
        assert_ne!(last_block.block_number, 0);

        // TODO : Assert that the tx was correctly executed

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
        let TestRunner { chain_config, db_service: db, dummy_contract: contract, eth_client, anvil: _anvil } =
            setup_test_env.await;

        // Start worker
        let worker_handle = {
            let db = Arc::clone(&db);
            tokio::spawn(async move { sync(db.backend(), &eth_client, &chain_config.chain_id).await })
        };

        let _ = contract.setIsCanceled(false).send().await;
        let _ = contract.fireEvent().send().await.expect("Failed to fire event");
        tokio::time::sleep(Duration::from_secs(5)).await;
        let last_block =
            db.backend().messaging_last_synced_l1_block_with_event().expect("failed to retrieve block").unwrap();
        assert_ne!(last_block.block_number, 0);

        // Send the event a second time
        let _ = contract.fireEvent().send().await.expect("Failed to fire event");
        tokio::time::sleep(Duration::from_secs(5)).await;
        // Assert that the last event in db is still the same as it is already processed (same nonce)
        assert_eq!(
            last_block.block_number,
            db.backend()
                .messaging_last_synced_l1_block_with_event()
                .expect("failed to retrieve block")
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
        let TestRunner { chain_config, db_service: db, dummy_contract: contract, eth_client, anvil: _anvil } =
            setup_test_env.await;

        // Start worker
        let worker_handle = {
            let db = Arc::clone(&db);
            tokio::spawn(async move { sync(db.backend(), &eth_client, &chain_config.chain_id).await })
        };

        // Mock cancelled message
        let _ = contract.setIsCanceled(true).send().await;
        let _ = contract.fireEvent().send().await.expect("Failed to fire event");
        tokio::time::sleep(Duration::from_secs(5)).await;
        let last_block =
            db.backend().messaging_last_synced_l1_block_with_event().expect("failed to retrieve block").unwrap();
        assert_eq!(last_block.block_number, 0);
        assert!(logs_contain("L1 Message was cancelled in block at timestamp : 0x66b4f105"));

        worker_handle.abort();
    }

    /// Test taken from starknet.rs to ensure consistency
    /// https://github.com/xJonathanLEI/starknet-rs/blob/2ddc69479d326ed154df438d22f2d720fbba746e/starknet-core/src/types/msg.rs#L96
    #[test]
    fn test_msg_to_l2_hash() {
        let msg = get_l1_to_l2_msg_hash(&LogMessageToL2 {
            fromAddress: Address::from_hex("c3511006C04EF1d78af4C8E0e74Ec18A6E64Ff9e").unwrap(),
            toAddress: felt_to_u256(
                Felt::from_hex("0x73314940630fd6dcda0d772d4c972c4e0a9946bef9dabf4ef84eda8ef542b82").unwrap(),
            ),
            selector: felt_to_u256(
                Felt::from_hex("0x2d757788a8d8d6f21d1cd40bce38a8222d70654214e96ff95d8086e684fbee5").unwrap(),
            ),
            payload: vec![
                felt_to_u256(
                    Felt::from_hex("0x689ead7d814e51ed93644bc145f0754839b8dcb340027ce0c30953f38f55d7").unwrap(),
                ),
                felt_to_u256(Felt::from_hex("0x2c68af0bb140000").unwrap()),
                felt_to_u256(Felt::from_hex("0x0").unwrap()),
            ],
            nonce: U256::from(775628),
            fee: U256::ZERO,
        })
        .expect("Failed to compute l1 to l2 msg hash");

        let expected_hash =
            <[u8; 32]>::from_hex("c51a543ef9563ad2545342b390b67edfcddf9886aa36846cf70382362fc5fab3").unwrap();

        assert_eq!(msg.0, expected_hash);
    }
}
