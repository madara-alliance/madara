use crate::client::StarknetCoreContract::LogMessageToL2;
use crate::client::{EthereumClient, StarknetCoreContract};
use crate::utils::u256_to_felt;
use alloy::eips::BlockNumberOrTag;
use alloy::primitives::{keccak256, FixedBytes, U256};
use alloy::sol_types::SolValue;
use anyhow::Context;
use futures::StreamExt;
use mc_db::{l1_db::LastSyncedEventBlock, MadaraBackend};
use mc_mempool::{Mempool, MempoolProvider};
use mp_utils::service::ServiceContext;
use starknet_api::core::{ChainId, ContractAddress, EntryPointSelector, Nonce};
use starknet_api::transaction::{Calldata, L1HandlerTransaction, TransactionVersion};
use starknet_types_core::felt::Felt;
use std::sync::Arc;

impl EthereumClient {
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
    pub async fn get_l1_to_l2_message_cancellations(&self, msg_hash: FixedBytes<32>) -> anyhow::Result<Felt> {
        //l1ToL2MessageCancellations
        let cancellation_timestamp = self.l1_core_contract.l1ToL2MessageCancellations(msg_hash).call().await?;
        u256_to_felt(cancellation_timestamp._0)
    }
}

pub async fn sync(
    backend: Arc<MadaraBackend>,
    client: Arc<EthereumClient>,
    chain_id: ChainId,
    mempool: Arc<Mempool>,
    mut ctx: ServiceContext,
) -> anyhow::Result<()> {
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
        .to_block(BlockNumberOrTag::Finalized)
        .watch()
        .await
        .context(
            "Failed to watch event filter - Ensure you are using an L1 RPC endpoint that points to an archive node",
        )?
        .into_stream();

    while let Some(Some(event_result)) = ctx.run_until_cancelled(event_stream.next()).await {
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
                let tx_nonce = Nonce(u256_to_felt(event.nonce)?);
                // cancelled message nonce should be inserted to avoid reprocessing
                match backend.has_l1_messaging_nonce(tx_nonce) {
                    Ok(false) => {
                        backend.set_l1_messaging_nonce(tx_nonce)?;
                    }
                    Ok(true) => {}
                    Err(e) => {
                        tracing::error!("⟠ Unexpected DB error: {:?}", e);
                        return Err(e.into());
                    }
                };
                continue;
            }

            match process_l1_message(&backend, &event, &meta.block_number, &meta.log_index, &chain_id, mempool.clone())
                .await
            {
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
    backend: &MadaraBackend,
    event: &LogMessageToL2,
    l1_block_number: &Option<u64>,
    event_index: &Option<u64>,
    _chain_id: &ChainId,
    mempool: Arc<Mempool>,
) -> anyhow::Result<Option<Felt>> {
    let transaction = parse_handle_l1_message_transaction(event)?;
    let tx_nonce = transaction.nonce;
    let fees: u128 = event.fee.try_into()?;

    // Ensure that L1 message has not been executed
    match backend.has_l1_messaging_nonce(tx_nonce) {
        Ok(true) => {
            tracing::debug!("⟠ Event already processed: {:?}", transaction);
            return Ok(None);
        }
        Err(e) => {
            tracing::error!("⟠ Unexpected DB error: {:?}", e);
            return Err(e.into());
        }
        _ => {}
    };

    let res = mempool.tx_accept_l1_handler(transaction.into(), fees)?;

    // HERMAN TODO: Actually this should be updated after the tx l1 handler is executed
    backend.set_l1_messaging_nonce(tx_nonce)?;

    // TODO: remove unwraps
    // Ques: shall it panic if no block number of event_index?
    let block_sent = LastSyncedEventBlock::new(l1_block_number.unwrap(), event_index.unwrap());
    backend.messaging_update_last_synced_l1_block_with_event(block_sent)?;

    Ok(Some(res.transaction_hash))
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

    let event_payload = event.payload.clone().into_iter().map(u256_to_felt).collect::<anyhow::Result<Vec<_>>>()?;

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
mod l1_messaging_tests {
    use super::*;

    use std::{sync::Arc, time::Duration};

    use crate::l1_messaging::sync;
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
    use blockifier::transaction::transaction_execution::Transaction;
    use mc_db::DatabaseService;
    use mc_mempool::{GasPriceProvider, L1DataProvider, Mempool, MempoolLimits};
    use mp_chain_config::ChainConfig;
    use mp_utils::service::ServiceContext;
    use rstest::*;
    use starknet_api::core::Nonce;
    use starknet_types_core::felt::Felt;
    use tempfile::TempDir;
    use tracing_test::traced_test;
    use url::Url;

    use self::DummyContract::DummyContractInstance;

    struct TestRunner {
        #[allow(dead_code)]
        anvil: AnvilInstance, // Not used but needs to stay in scope otherwise it will be dropped
        chain_config: Arc<ChainConfig>,
        db_service: Arc<DatabaseService>,
        dummy_contract: DummyContractInstance<Http<Client>, RootProvider<Http<Client>>>,
        eth_client: EthereumClient,
        mempool: Arc<Mempool>,
    }

    // LogMessageToL2 from https://etherscan.io/tx/0x21980d6674d33e50deee43c6c30ef3b439bd148249b4539ce37b7856ac46b843
    // bytecode is compiled DummyContractBasicTestCase using:
    //  $ solc --version
    // solc, the solidity compiler commandline interface
    // Version: 0.8.26
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
    /// 7. TODO : Assert that the tx is succesfully submited to the mempool
    /// 8. Assert that the event is successfully pushed to the db
    /// 9. TODO : Assert that the tx was correctly executed
    #[fixture]
    async fn setup_test_env() -> TestRunner {
        // Start Anvil instance
        let anvil = Anvil::new().block_time(1).chain_id(1337).try_spawn().expect("failed to spawn anvil instance");
        println!("Anvil started and running at `{}`", anvil.endpoint());

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

        let l1_gas_setter = GasPriceProvider::new();
        let l1_data_provider: Arc<dyn L1DataProvider> = Arc::new(l1_gas_setter.clone());

        let mempool = Arc::new(Mempool::new(
            Arc::clone(db.backend()),
            Arc::clone(&l1_data_provider),
            MempoolLimits::for_testing(),
        ));

        // Set up metrics service
        let l1_block_metrics = L1BlockMetrics::register().unwrap();

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

        TestRunner { anvil, chain_config, db_service: db, dummy_contract: contract, eth_client, mempool }
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
        let TestRunner { chain_config, db_service: db, dummy_contract: contract, eth_client, anvil: _anvil, mempool } =
            setup_test_env.await;

        // Start worker
        let worker_handle = {
            let db = Arc::clone(&db);
            let mempool = mempool.clone();
            tokio::spawn(async move {
                sync(
                    Arc::clone(db.backend()),
                    Arc::new(eth_client),
                    chain_config.chain_id.clone(),
                    mempool,
                    ServiceContext::new_for_testing(),
                )
                .await
            })
        };

        let _ = contract.setIsCanceled(false).send().await;
        // Send a Event and wait for processing, Panic if fail
        let _ = contract.fireEvent().send().await.expect("Failed to fire event");
        tokio::time::sleep(Duration::from_secs(5)).await;

        let expected_nonce = Nonce(Felt::from_dec_str("0").expect("failed to parse nonce string"));

        let current_nonce = mempool.backend.get_l1_messaging_nonce_latest().unwrap().unwrap();
        assert_eq!(current_nonce, expected_nonce);

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
        assert!(logs_contain("fromAddress: 0xae0ee0a63a2ce6baeeffe56e7714fb4efe48d419"));

        // Assert the tx hash computed by the worker is correct
        assert!(logs_contain(
            format!("event hash : {:?}", contract.getL1ToL2MsgHash().call().await.expect("failed to get hash")._0)
                .as_str()
        ));

        // Assert that the event is well stored in db
        let last_block =
            db.backend().messaging_last_synced_l1_block_with_event().expect("failed to retrieve block").unwrap();
        assert_ne!(last_block.block_number, 0);

        assert!(db.backend().has_l1_messaging_nonce(expected_nonce).unwrap());
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
        let TestRunner { chain_config, db_service: db, dummy_contract: contract, eth_client, anvil: _anvil, mempool } =
            setup_test_env.await;

        // Start worker
        let worker_handle = {
            let db = Arc::clone(&db);
            tokio::spawn(async move {
                sync(
                    Arc::clone(db.backend()),
                    Arc::new(eth_client),
                    chain_config.chain_id.clone(),
                    mempool,
                    ServiceContext::new_for_testing(),
                )
                .await
            })
        };

        let _ = contract.setIsCanceled(false).send().await;
        let _ = contract.fireEvent().send().await.expect("Failed to fire event");
        tokio::time::sleep(Duration::from_secs(5)).await;
        let last_block =
            db.backend().messaging_last_synced_l1_block_with_event().expect("failed to retrieve block").unwrap();
        assert_ne!(last_block.block_number, 0);
        let nonce = Nonce(Felt::from_dec_str("10000000000000000").expect("failed to parse nonce string"));
        assert!(db.backend().has_l1_messaging_nonce(nonce).unwrap());

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
        let TestRunner { chain_config, db_service: db, dummy_contract: contract, eth_client, anvil: _anvil, mempool } =
            setup_test_env.await;

        // Start worker
        let worker_handle = {
            let db = Arc::clone(&db);
            tokio::spawn(async move {
                sync(
                    Arc::clone(db.backend()),
                    Arc::new(eth_client),
                    chain_config.chain_id.clone(),
                    mempool,
                    ServiceContext::new_for_testing(),
                )
                .await
            })
        };

        // Mock cancelled message
        let _ = contract.setIsCanceled(true).send().await;
        let _ = contract.fireEvent().send().await.expect("Failed to fire event");
        tokio::time::sleep(Duration::from_secs(5)).await;
        let last_block =
            db.backend().messaging_last_synced_l1_block_with_event().expect("failed to retrieve block").unwrap();
        assert_eq!(last_block.block_number, 0);
        let nonce = Nonce(Felt::from_dec_str("10000000000000000").expect("failed to parse nonce string"));
        // cancelled message nonce should be inserted to avoid reprocessing
        assert!(db.backend().has_l1_messaging_nonce(nonce).unwrap());
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
