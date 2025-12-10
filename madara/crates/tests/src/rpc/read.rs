// source of truth for the data here is the juno rpc. for each method tested here, same call was
// made to the juno rpc.

#[cfg(test)]
mod test_rpc_read_calls {
    use crate::{MadaraCmd, MadaraCmdBuilder};
    use flate2::read::GzDecoder;
    use rstest::rstest;
    use starknet_core::types::Felt;
    use starknet_core::types::{
        BlockHashAndNumber, BlockId, BlockStatus, BlockWithReceipts, BlockWithTxHashes, BlockWithTxs,
        BroadcastedDeployAccountTransactionV3, BroadcastedTransaction, ContractClass, ContractStorageDiffItem,
        DataAvailabilityMode, DeclareTransaction, DeclareTransactionReceipt, DeclareTransactionV0, EmittedEvent,
        EthAddress, EventFilter, EventsPage, ExecutionResources, ExecutionResult, FeeEstimate, FeePayment,
        FunctionCall, L1DataAvailabilityMode, L1HandlerTransaction, MaybePreConfirmedBlockWithReceipts,
        MaybePreConfirmedBlockWithTxHashes, MaybePreConfirmedBlockWithTxs, MaybePreConfirmedStateUpdate,
        MessageFeeEstimate, MsgFromL1, PriceUnit, ReceiptBlock, ResourceBounds, ResourceBoundsMapping, ResourcePrice,
        SimulationFlagForEstimateFee, StateDiff, StateUpdate, StorageEntry, Transaction, TransactionFinalityStatus,
        TransactionReceipt, TransactionReceiptWithBlockInfo, TransactionStatus, TransactionWithReceipt,
    };
    use starknet_providers::Provider;
    use std::any::Any;
    use std::fmt::Write;
    use std::io::Read;
    use tokio::sync::OnceCell;

    static MADARA_INSTANCE: OnceCell<MadaraCmd> = OnceCell::const_new();

    pub async fn get_madara() -> &'static MadaraCmd {
        MADARA_INSTANCE
            .get_or_init(|| async {
                let mut madara = MadaraCmdBuilder::new()
                    .args(["--full", "--network", "sepolia", "--sync-stop-at", "19", "--no-l1-sync"])
                    .run();

                madara.wait_for_ready().await;
                madara.wait_for_sync_to(19).await;
                madara
            })
            .await
    }

    /// Fetches the latest block hash and number.
    ///
    /// Example curl command:
    ///
    /// ```bash
    /// curl --location 'https://free-rpc.nethermind.io/sepolia-juno/' \
    /// --header 'Content-Type: application/json' \
    /// --data '{
    ///     "jsonrpc": "2.0",
    ///     "method": "starknet_blockHashAndNumber",
    ///     "params": {},
    ///     "id": 1
    /// }'
    /// ```
    #[rstest]
    #[tokio::test]
    async fn test_block_hash_and_number_works() {
        let madara = get_madara().await;
        let json_client = madara.json_rpc();
        let result = { json_client.block_hash_and_number().await.unwrap() };
        assert_eq!(
            result,
            BlockHashAndNumber {
                // https://sepolia.voyager.online/block/19
                block_hash: Felt::from_hex("0x4177d1ba942a4ab94f86a476c06f0f9e02363ad410cdf177c54064788c9bcb5")
                    .unwrap(),
                block_number: 19
            }
        );
    }

    /// Retrieves the number of transactions in a specific block.
    ///
    /// Example curl command:
    ///
    /// ```bash
    /// curl --location 'https://free-rpc.nethermind.io/sepolia-juno/' \
    /// --header 'Content-Type: application/json' \
    /// --data '{
    ///     "jsonrpc": "2.0",
    ///     "method": "starknet_getBlockTransactionCount",
    ///     "params": {
    ///         "block_id": {
    ///             "block_number": 2
    ///         }
    ///     },
    ///     "id": 1
    /// }'
    /// ```
    #[rstest]
    #[tokio::test]
    async fn test_get_block_txn_count_works() {
        let madara = get_madara().await;
        let json_client = madara.json_rpc();
        let result = { json_client.get_block_transaction_count(BlockId::Number(2)).await.unwrap() };
        assert_eq!(result, 1);
    }

    /// Fetches the latest block hash and number.
    ///
    /// Example curl command:
    ///
    /// ```bash
    /// curl --location 'https://free-rpc.nethermind.io/sepolia-juno/' \
    /// --header 'Content-Type: application/json' \
    /// --data '[
    ///     {
    ///         "jsonrpc": "2.0",
    ///         "method": "starknet_blockHashAndNumber",
    ///         "params": {},
    ///         "id": 0
    ///     },
    ///     {
    ///         "jsonrpc": "2.0",
    ///         "method": "starknet_getBlockTransactionCount",
    ///         "params": {
    ///             "block_id": {
    ///                 "block_number": 2
    ///             }
    ///         },
    ///         "id": 1
    ///     }
    /// ]'
    /// ```
    #[rstest]
    #[tokio::test]
    async fn test_batched_requests_work() {
        let madara = get_madara().await;

        // use reqwest to send a batch request to the madara rpc.
        // TODO: use a jsonrpc client instead of reqwest when we move
        // to starknet-providers 0.12.0
        let client = reqwest::Client::new();
        let res = client
            .post(madara.rpc_url.clone().unwrap())
            .json(&[
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "starknet_blockHashAndNumber",
                    "params": {},
                    "id": 0
                }),
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "starknet_getBlockTransactionCount",
                    "params": {
                        "block_id": {
                            "block_number": 2
                        }
                    },
                    "id": 1
                }),
            ])
            .send()
            .await
            .unwrap();

        let result = res.json::<serde_json::Value>().await.unwrap();

        assert_eq!(
            result[0],
            serde_json::json!({
                "jsonrpc": "2.0",
                "result": {
                    "block_hash": "0x4177d1ba942a4ab94f86a476c06f0f9e02363ad410cdf177c54064788c9bcb5",
                    "block_number": 19
                },
                "id": 0
            })
        );
        assert_eq!(
            result[1],
            serde_json::json!({
                "jsonrpc": "2.0",
                "result": 1,
                "id": 1
            })
        );
    }

    /// Fetches a block with its transactions and receipts.
    ///
    /// Example curl command:
    ///
    /// ```bash
    /// curl --location 'https://free-rpc.nethermind.io/sepolia-juno/' \
    /// --header 'Content-Type: application/json' \
    /// --data '{
    ///     "jsonrpc": "2.0",
    ///     "method": "starknet_getBlockWithReceipts",
    ///     "params": {
    ///         "block_id": {
    ///             "block_number": 2
    ///         }
    ///     },
    ///     "id": 1
    /// }'
    /// ```
    // FIXME: Ignoring this test because the starknet-rs library does not comply with the specifications for receipts.
    // Specifically, it includes the TransactionHash in the receipt, which is against the expected standard.
    // Issue: https://github.com/xJonathanLEI/starknet-rs/issues/678
    #[ignore]
    #[rstest]
    #[tokio::test]
    async fn test_get_block_txn_with_receipts_works() {
        let madara = get_madara().await;
        let json_client = madara.json_rpc();
        let block = json_client
            .get_block_with_receipts(BlockId::Number(2))
            .await
            .expect("Failed to get block with receipts for block number 2");

        let expected_block = MaybePreConfirmedBlockWithReceipts::Block(BlockWithReceipts {
            status: BlockStatus::AcceptedOnL2,
            block_hash: Felt::from_hex("0x7a906dfd1ff77a121b8048e6f750cda9e949d341c4487d4c6a449f183f0e61d").unwrap(),
            parent_hash: Felt::from_hex("0x78b67b11f8c23850041e11fb0f3b39db0bcb2c99d756d5a81321d1b483d79f6").unwrap(),
            block_number: 2,
            new_root: Felt::from_hex("0xe005205a1327f3dff98074e528f7b96f30e0624a1dfcf571bdc81948d150a0").unwrap(),
            timestamp: 1700475581,
            sequencer_address: Felt::from_hex("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8")
                .unwrap(),
            l1_gas_price: ResourcePrice {
                price_in_fri: Felt::from_hex("0x0").unwrap(),
                price_in_wei: Felt::from_hex("0x3b9ad016").unwrap(),
            },
            l1_data_gas_price: ResourcePrice {
                price_in_fri: Felt::from_hex("0x1").unwrap(),
                price_in_wei: Felt::from_hex("0x1").unwrap(),
            },
            l2_gas_price: ResourcePrice {
                price_in_fri: Felt::from_hex("0x1").unwrap(),
                price_in_wei: Felt::from_hex("0x1").unwrap(),
            },
            l1_da_mode: L1DataAvailabilityMode::Calldata,
            starknet_version: "0.12.3".to_string(),
            transactions: vec![TransactionWithReceipt {
                transaction: Transaction::Declare(DeclareTransaction::V0(DeclareTransactionV0 {
                    transaction_hash: Felt::from_hex(
                        "0x701d9adb9c60bc2fd837fe3989e15aeba4be1a6e72bb6f61ffe35a42866c772",
                    )
                    .unwrap(),
                    sender_address: Felt::from_hex("0x1").unwrap(),
                    max_fee: Felt::from_hex("0x0").unwrap(),
                    signature: vec![],
                    class_hash: Felt::from_hex("0x4f23a756b221f8ce46b72e6a6b10ee7ee6cf3b59790e76e02433104f9a8c5d1")
                        .unwrap(),
                }))
                .into(),
                receipt: {
                    TransactionReceipt::Declare(DeclareTransactionReceipt {
                        transaction_hash: Felt::from_hex(
                            "0x701d9adb9c60bc2fd837fe3989e15aeba4be1a6e72bb6f61ffe35a42866c772",
                        )
                        .unwrap(),
                        actual_fee: FeePayment { amount: Felt::from_hex("0x0").unwrap(), unit: PriceUnit::Wei },
                        finality_status: TransactionFinalityStatus::AcceptedOnL2,
                        messages_sent: vec![],
                        events: vec![],
                        execution_resources: ExecutionResources {
                            l1_gas: Default::default(),
                            l1_data_gas: Default::default(),
                            l2_gas: Default::default(),
                        },
                        execution_result: ExecutionResult::Succeeded,
                    })
                },
            }],
        });
        assert_eq!(block, expected_block);
    }

    /// Retrieves a block with transaction hashes.
    ///
    /// Example curl command:
    ///
    /// ```bash
    /// curl --location 'https://free-rpc.nethermind.io/sepolia-juno/' \
    /// --header 'Content-Type: application/json' \
    /// --data '{
    ///     "jsonrpc": "2.0",
    ///     "method": "starknet_getBlockWithTxHashes",
    ///     "params": {
    ///         "block_id": {
    ///             "block_number": 2
    ///         }
    ///     },
    ///     "id": 1
    /// }'
    /// ```
    #[rstest]
    #[tokio::test]
    async fn test_get_block_txn_with_tx_hashes_works() {
        let madara = get_madara().await;
        let json_client = madara.json_rpc();
        let block = { json_client.get_block_with_tx_hashes(BlockId::Number(2)).await.unwrap() };

        let expected_block = MaybePreConfirmedBlockWithTxHashes::Block(BlockWithTxHashes {
            status: BlockStatus::AcceptedOnL2,
            block_hash: Felt::from_hex("0x7a906dfd1ff77a121b8048e6f750cda9e949d341c4487d4c6a449f183f0e61d").unwrap(),
            parent_hash: Felt::from_hex("0x78b67b11f8c23850041e11fb0f3b39db0bcb2c99d756d5a81321d1b483d79f6").unwrap(),
            block_number: 2,
            new_root: Felt::from_hex("0xe005205a1327f3dff98074e528f7b96f30e0624a1dfcf571bdc81948d150a0").unwrap(),
            timestamp: 1700475581,
            sequencer_address: Felt::from_hex("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8")
                .unwrap(),
            l1_gas_price: ResourcePrice {
                price_in_fri: Felt::from_hex("0x0").unwrap(),
                price_in_wei: Felt::from_hex("0x3b9ad016").unwrap(),
            },
            l1_data_gas_price: ResourcePrice {
                price_in_fri: Felt::from_hex("0x1").unwrap(),
                price_in_wei: Felt::from_hex("0x1").unwrap(),
            },
            l2_gas_price: ResourcePrice {
                price_in_fri: Felt::from_hex("0x1").unwrap(),
                price_in_wei: Felt::from_hex("0x1").unwrap(),
            },
            l1_da_mode: L1DataAvailabilityMode::Calldata,
            starknet_version: "0.12.3".to_string(),
            transactions: vec![
                Felt::from_hex("0x701d9adb9c60bc2fd837fe3989e15aeba4be1a6e72bb6f61ffe35a42866c772").unwrap()
            ],
        });
        assert_eq!(block, expected_block);
    }

    /// Fetches a block with full transaction details.
    ///
    /// Example curl command:
    ///
    /// ```bash
    /// curl --location 'https://free-rpc.nethermind.io/sepolia-juno/' \
    /// --header 'Content-Type: application/json' \
    /// --data '{
    ///     "jsonrpc": "2.0",
    ///     "method": "starknet_getBlockWithTxs",
    ///     "params": {
    ///         "block_id": {
    ///             "block_number": 2
    ///         }
    ///     },
    ///     "id": 1
    /// }'
    /// ```
    #[rstest]
    #[tokio::test]
    async fn test_get_block_txn_with_tx_works() {
        let madara = get_madara().await;
        let json_client = madara.json_rpc();
        let block = json_client.get_block_with_txs(BlockId::Number(2)).await.unwrap();

        let expected_block = MaybePreConfirmedBlockWithTxs::Block(BlockWithTxs {
            status: BlockStatus::AcceptedOnL2,
            block_hash: Felt::from_hex("0x7a906dfd1ff77a121b8048e6f750cda9e949d341c4487d4c6a449f183f0e61d").unwrap(),
            parent_hash: Felt::from_hex("0x78b67b11f8c23850041e11fb0f3b39db0bcb2c99d756d5a81321d1b483d79f6").unwrap(),
            block_number: 2,
            new_root: Felt::from_hex("0xe005205a1327f3dff98074e528f7b96f30e0624a1dfcf571bdc81948d150a0").unwrap(),
            timestamp: 1700475581,
            sequencer_address: Felt::from_hex("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8")
                .unwrap(),
            l1_gas_price: ResourcePrice {
                price_in_fri: Felt::from_hex("0x0").unwrap(),
                price_in_wei: Felt::from_hex("0x3b9ad016").unwrap(),
            },
            l1_data_gas_price: ResourcePrice {
                price_in_fri: Felt::from_hex("0x1").unwrap(),
                price_in_wei: Felt::from_hex("0x1").unwrap(),
            },
            l2_gas_price: ResourcePrice {
                price_in_fri: Felt::from_hex("0x1").unwrap(),
                price_in_wei: Felt::from_hex("0x1").unwrap(),
            },
            l1_da_mode: L1DataAvailabilityMode::Calldata,
            starknet_version: "0.12.3".to_string(),
            transactions: vec![Transaction::Declare(DeclareTransaction::V0(DeclareTransactionV0 {
                transaction_hash: Felt::from_hex("0x701d9adb9c60bc2fd837fe3989e15aeba4be1a6e72bb6f61ffe35a42866c772")
                    .unwrap(),
                sender_address: Felt::from_hex("0x1").unwrap(),
                max_fee: Felt::from_hex("0x0").unwrap(),
                signature: vec![],
                class_hash: Felt::from_hex("0x4f23a756b221f8ce46b72e6a6b10ee7ee6cf3b59790e76e02433104f9a8c5d1")
                    .unwrap(),
            }))],
        });
        assert_eq!(block, expected_block);
    }

    /// Retrieves the class hash at a specific block and contract address.
    ///
    /// Example curl command:
    ///
    /// ```bash
    /// curl --location 'https://free-rpc.nethermind.io/sepolia-juno/' \
    /// --header 'Content-Type: application/json' \
    /// --data '{
    ///     "jsonrpc": "2.0",
    ///     "method": "starknet_getClassHashAt",
    ///     "params": {
    ///         "block_id": {
    ///             "block_number": 15
    ///         },
    ///         "contract_address": "0x04c5772d1914fe6ce891b64eb35bf3522aeae1315647314aac58b01137607f3f"
    ///     },
    ///     "id": 1
    /// }'
    /// ```
    #[rstest]
    #[tokio::test]
    async fn test_get_class_hash_at_works() {
        let madara = get_madara().await;
        let json_client = madara.json_rpc();
        let class_hash = {
            json_client
                .get_class_hash_at(
                    BlockId::Number(15),
                    Felt::from_hex("0x04c5772d1914fe6ce891b64eb35bf3522aeae1315647314aac58b01137607f3f").unwrap(),
                )
                .await
                .unwrap()
        };
        let expected_class_hash =
            Felt::from_hex("0xd0e183745e9dae3e4e78a8ffedcce0903fc4900beace4e0abf192d4c202da3").unwrap();

        assert_eq!(class_hash, expected_class_hash);
    }

    /// Retrieves the nonce for a specific contract address at a given block.
    ///
    /// Example curl command:
    ///
    /// ```bash
    /// curl --location 'https://free-rpc.nethermind.io/sepolia-juno/' \
    /// --header 'Content-Type: application/json' \
    /// --data '{
    ///     "jsonrpc": "2.0",
    ///     "method": "starknet_getNonce",
    ///     "params": {
    ///         "block_id": {
    ///             "block_number": 15
    ///         },
    ///         "contract_address": "0x0535ca4e1d1be7ec4a88d51a2962cd6c5aea1be96cb2c0b60eb1721dc34f800d"
    ///     },
    ///     "id": 1
    /// }'
    /// ```
    #[rstest]
    #[tokio::test]
    async fn test_get_nonce_works() {
        let madara = get_madara().await;
        let json_client = madara.json_rpc();
        let nonce = {
            json_client
                .get_nonce(
                    BlockId::Number(19),
                    Felt::from_hex("0x0535ca4e1d1be7ec4a88d51a2962cd6c5aea1be96cb2c0b60eb1721dc34f800d").unwrap(),
                )
                .await
                .unwrap()
        };
        let expected_nonce = Felt::from_hex("0x2").unwrap();

        assert_eq!(nonce, expected_nonce);
    }

    /// Retrieves a transaction by its block ID and index.
    ///
    /// Example curl command:
    ///
    /// ```bash
    /// curl --location 'https://free-rpc.nethermind.io/sepolia-juno/' \
    /// --header 'Content-Type: application/json' \
    /// --data '{
    ///     "jsonrpc": "2.0",
    ///     "method": "starknet_getTransactionByBlockIdAndIndex",
    ///     "params": {
    ///         "block_id": {
    ///             "block_number": 16
    ///         },
    ///         "index": 1
    ///     },
    ///     "id": 1
    /// }'
    /// ```
    #[rstest]
    #[tokio::test]
    async fn test_get_txn_by_block_id_and_index_works() {
        let madara = get_madara().await;
        let json_client = madara.json_rpc();
        let txn = { json_client.get_transaction_by_block_id_and_index(BlockId::Number(16), 1).await.unwrap() };
        let expected_txn = Transaction::L1Handler(L1HandlerTransaction {
            transaction_hash: Felt::from_hex("0x68fa87ed202095170a2f551017bf646180f43f4687553dc45e61598349a9a8a")
                .unwrap(),
            version: Felt::from_hex("0x0").unwrap(),
            nonce: 11,
            contract_address: Felt::from_hex("0x4c5772d1914fe6ce891b64eb35bf3522aeae1315647314aac58b01137607f3f")
                .unwrap(),
            entry_point_selector: Felt::from_hex("0x2d757788a8d8d6f21d1cd40bce38a8222d70654214e96ff95d8086e684fbee5")
                .unwrap(),
            calldata: vec![
                Felt::from_hex("0x8453fc6cd1bcfe8d4dfc069c400b433054d47bdc").unwrap(),
                Felt::from_hex("0x70503f026c7af73cfd2b007fe650e8c310256e9674ac4e42797c291edca5e84").unwrap(),
                Felt::from_hex("0x15fb7f9b8c38000").unwrap(),
                Felt::from_hex("0x0").unwrap(),
            ],
        });

        assert_eq!(txn, expected_txn);
    }

    /// Retrieves a transaction by its hash.
    ///
    /// Example curl command:
    ///
    /// ```bash
    /// curl --location 'https://free-rpc.nethermind.io/sepolia-juno/' \
    /// --header 'Content-Type: application/json' \
    /// --data '{
    ///     "jsonrpc": "2.0",
    ///     "method": "starknet_getTransactionByHash",
    ///     "params": {
    ///         "transaction_hash": "0x68fa87ed202095170a2f551017bf646180f43f4687553dc45e61598349a9a8a"
    ///     },
    ///     "id": 1
    /// }'
    /// ```
    #[rstest]
    #[tokio::test]
    async fn test_get_txn_by_hash_works() {
        let madara = get_madara().await;
        let json_client = madara.json_rpc();
        let txn = {
            json_client
                .get_transaction_by_hash(
                    Felt::from_hex("0x68fa87ed202095170a2f551017bf646180f43f4687553dc45e61598349a9a8a").unwrap(),
                )
                .await
                .unwrap()
        };
        let expected_txn = Transaction::L1Handler(L1HandlerTransaction {
            transaction_hash: Felt::from_hex("0x68fa87ed202095170a2f551017bf646180f43f4687553dc45e61598349a9a8a")
                .unwrap(),
            version: Felt::from_hex("0x0").unwrap(),
            nonce: 11,
            contract_address: Felt::from_hex("0x4c5772d1914fe6ce891b64eb35bf3522aeae1315647314aac58b01137607f3f")
                .unwrap(),
            entry_point_selector: Felt::from_hex("0x2d757788a8d8d6f21d1cd40bce38a8222d70654214e96ff95d8086e684fbee5")
                .unwrap(),
            calldata: vec![
                Felt::from_hex("0x8453fc6cd1bcfe8d4dfc069c400b433054d47bdc").unwrap(),
                Felt::from_hex("0x70503f026c7af73cfd2b007fe650e8c310256e9674ac4e42797c291edca5e84").unwrap(),
                Felt::from_hex("0x15fb7f9b8c38000").unwrap(),
                Felt::from_hex("0x0").unwrap(),
            ],
        });

        assert_eq!(txn, expected_txn);
    }

    /// Retrieves the receipt for a transaction.
    ///
    /// Example curl command:
    ///
    /// ```bash
    /// curl --location 'https://free-rpc.nethermind.io/sepolia-juno/' \
    /// --header 'Content-Type: application/json' \
    /// --data '{
    ///     "jsonrpc": "2.0",
    ///     "method": "starknet_getTransactionReceipt",
    ///     "params": {
    ///         "transaction_hash": "0x701d9adb9c60bc2fd837fe3989e15aeba4be1a6e72bb6f61ffe35a42866c772"
    ///     },
    ///     "id": 1
    /// }'
    /// ```
    #[rstest]
    #[tokio::test]
    // TODO: replace this with jsonrpsee client
    async fn test_get_txn_receipt_works() {
        let madara = get_madara().await;
        let json_client = madara.json_rpc();
        let txn_receipt = {
            json_client
                .get_transaction_receipt(
                    Felt::from_hex("0x701d9adb9c60bc2fd837fe3989e15aeba4be1a6e72bb6f61ffe35a42866c772").unwrap(),
                )
                .await
                .unwrap()
        };
        let expected_txn_receipt = TransactionReceiptWithBlockInfo {
            receipt: TransactionReceipt::Declare(DeclareTransactionReceipt {
                transaction_hash: Felt::from_hex("0x701d9adb9c60bc2fd837fe3989e15aeba4be1a6e72bb6f61ffe35a42866c772")
                    .unwrap(),
                actual_fee: FeePayment { amount: Felt::from_hex("0x0").unwrap(), unit: PriceUnit::Wei },
                finality_status: TransactionFinalityStatus::AcceptedOnL2,
                messages_sent: vec![],
                events: vec![],
                execution_resources: ExecutionResources { l1_gas: 0, l2_gas: 0, l1_data_gas: 0 },
                execution_result: ExecutionResult::Succeeded,
            }),
            block: ReceiptBlock::Block {
                block_hash: Felt::from_hex("0x7a906dfd1ff77a121b8048e6f750cda9e949d341c4487d4c6a449f183f0e61d")
                    .unwrap(),
                block_number: 2,
            },
        };

        assert_eq!(txn_receipt, expected_txn_receipt);
    }

    /// Retrieves the status of a transaction.
    ///
    /// Example curl command:
    ///
    /// ```bash
    /// curl --location 'https://free-rpc.nethermind.io/sepolia-juno/' \
    /// --header 'Content-Type: application/json' \
    /// --data '{
    ///     "jsonrpc": "2.0",
    ///     "method": "starknet_getTransactionStatus",
    ///     "params": {
    ///         "transaction_hash": "0x68fa87ed202095170a2f551017bf646180f43f4687553dc45e61598349a9a8a"
    ///     },
    ///     "id": 1
    /// }'
    /// ```
    ///
    /// Note: Juno endpoint will provide response with AcceptedOnL1, in the current case, we are not
    /// running madara in sync with L1. And only way to check whether what madara is storing is right
    /// as per the L1 is to sync all the nodes of the L2 and make a call to the core contract on L1 to
    /// verify the state root.
    ///
    /// Hence, all the txn would be marked as AcceptedOnL2.
    #[rstest]
    #[tokio::test]
    async fn test_get_txn_status_works() {
        let madara = get_madara().await;
        let json_client = madara.json_rpc();
        let txn_status = {
            json_client
                .get_transaction_status(
                    Felt::from_hex("0x68fa87ed202095170a2f551017bf646180f43f4687553dc45e61598349a9a8a").unwrap(),
                )
                .await
                .unwrap()
        };
        let expected_txn_status = TransactionStatus::AcceptedOnL2(ExecutionResult::Succeeded);

        // TODO: The shared madara state needs a rework as we only run these
        // tests with `--test-threads=1`. These tests
        // tokio::task::spawn_blocking(|| *MADARA.blocking_lock() = None);

        assert_eq!(txn_status, expected_txn_status);
    }

    /// Retrieves the storage value at a specific contract address, key, and block.
    ///
    /// Example curl command:
    ///
    /// ```bash
    /// curl --location 'https://free-rpc.nethermind.io/sepolia-juno/' \
    /// --header 'Content-Type: application/json' \
    /// --data '{
    ///     "jsonrpc": "2.0",
    ///     "method": "starknet_getStorageAt",
    ///     "params": {
    ///         "contract_address": "0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7",
    ///         "key": "0x0341c1bdfd89f69748aa00b5742b03adbffd79b8e80cab5c50d91cd8c2a79be1",
    ///         "block_id": {
    ///             "block_number": 15
    ///         }
    ///     },
    ///     "id": 1
    /// }'
    /// ```
    #[rstest]
    #[tokio::test]
    async fn test_get_storage_at_works() {
        let madara = get_madara().await;
        let json_client = madara.json_rpc();
        let storage_response = {
            json_client
                .get_storage_at(
                    Felt::from_hex("0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7").unwrap(),
                    Felt::from_hex("0x0341c1bdfd89f69748aa00b5742b03adbffd79b8e80cab5c50d91cd8c2a79be1").unwrap(),
                    BlockId::Number(12),
                )
                .await
                .unwrap()
        };
        let expected_storage_response = Felt::from_hex("0x4574686572").unwrap();

        assert_eq!(storage_response, expected_storage_response);
    }

    /// Retrieves the state update for a specific block.
    ///
    /// Example curl command:
    ///
    /// ```bash
    /// curl --location 'https://free-rpc.nethermind.io/sepolia-juno/' \
    /// --header 'Content-Type: application/json' \
    /// --data '{
    ///     "jsonrpc": "2.0",
    ///     "method": "starknet_getStateUpdate",
    ///     "params": {
    ///         "block_id": {
    ///             "block_number": 13
    ///         }
    ///     },
    ///     "id": 1
    /// }'
    /// ```
    #[rstest]
    #[tokio::test]
    async fn test_get_state_update_works() {
        let madara = get_madara().await;
        let json_client = madara.json_rpc();
        let state_update = json_client
            .get_state_update(BlockId::Number(13))
            .await
            .expect("Failed to get state update for block number 13");
        let state_update = match state_update {
            MaybePreConfirmedStateUpdate::Update(mut state_update) => {
                state_update.state_diff.storage_diffs.sort_by(|a, b| a.address.cmp(&b.address));
                MaybePreConfirmedStateUpdate::Update(state_update)
            }
            _ => unreachable!("State update at block 13 should not be pending"),
        };

        let expected_state_update = MaybePreConfirmedStateUpdate::Update(StateUpdate {
            block_hash: Felt::from_hex("0x12e2fe9e5273b777341a372edc56ca0327dc2237232cf2fed6cecc7398ffe9d").unwrap(),
            old_root: Felt::from_hex("0x7b6d0a312a1304bc1f99396c227a3bf062ff390258d2341309b4f60e6520bc9").unwrap(),
            new_root: Felt::from_hex("0x73ef61c78f5bda0bd3ef54d360484d06d32032e3b9287a71e0798526654a733").unwrap(),
            state_diff: StateDiff {
                storage_diffs: vec![
                    ContractStorageDiffItem {
                        address: Felt::from_hex("0x1").unwrap(),
                        storage_entries: vec![StorageEntry {
                            key: Felt::from_hex("0x3").unwrap(),
                            value: Felt::from_hex("0x37644818236ee05b7e3b180bed64ea70ee3dd1553ca334a5c2a290ee276f380")
                                .unwrap(),
                        }],
                    },
                    ContractStorageDiffItem {
                        address: Felt::from_hex("0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7")
                            .unwrap(),
                        storage_entries: vec![
                            StorageEntry {
                                key: Felt::from_hex(
                                    "0x110e2f729c9c2b988559994a3daccd838cf52faf88e18101373e67dd061455a",
                                )
                                .unwrap(),
                                value: Felt::from_hex("0xe20a99b3d590000").unwrap(),
                            },
                            StorageEntry {
                                key: Felt::from_hex(
                                    "0x6cfd3e69ed325a8ac721ef6c60099111df74d4c17f62221dc847b26c9e9db3d",
                                )
                                .unwrap(),
                                value: Felt::from_hex("0x71afd498d0000").unwrap(),
                            },
                        ],
                    },
                ],
                deprecated_declared_classes: vec![],
                declared_classes: vec![],
                deployed_contracts: vec![],
                replaced_classes: vec![],
                nonces: vec![],
                migrated_compiled_classes: None,
            },
        });

        assert_eq!(state_update, expected_state_update);
    }

    /// Retrieves events based on a filter.
    ///
    /// Example curl command:
    ///
    /// ```bash
    /// curl --location 'https://free-rpc.nethermind.io/sepolia-juno/' \
    /// --header 'Content-Type: application/json' \
    /// --data '{
    ///     "jsonrpc": "2.0",
    ///     "method": "starknet_getEvents",
    ///     "params": {
    ///         "filter": {
    ///             "from_block": {
    ///                 "block_number": 0
    ///             },
    ///             "to_block": {
    ///                 "block_number": 19
    ///             },
    ///             "address": "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7",
    ///             "keys": [
    ///                 []
    ///             ],
    ///             "continuation_token": "",
    ///             "chunk_size": 2
    ///         }
    ///     },
    ///     "id": 1
    /// }'
    /// ```
    ///
    /// Note: The response from juno contains different continuation token from madara. Only spec we
    /// need to follow is that the continuation token should be of type string.
    ///
    /// In here we are testing the event data apart from continuation token and checking type for
    /// the continuation token.
    #[rstest]
    #[tokio::test]
    async fn test_get_events_works() {
        let madara = get_madara().await;
        let json_client = madara.json_rpc();
        let events = {
            json_client
                .get_events(
                    EventFilter {
                        from_block: Some(BlockId::Number(0)),
                        to_block: Some(BlockId::Number(19)),
                        address: Some(
                            Felt::from_hex("0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7")
                                .unwrap(),
                        ),
                        keys: Some(vec![vec![]]),
                    },
                    None,
                    2,
                )
                .await
                .unwrap()
        };

        let expected_events = EventsPage {
            events: vec![
                EmittedEvent {
                    from_address: Felt::from_hex("0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7")
                        .unwrap(),
                    keys: vec![
                        Felt::from_hex("0x3774b0545aabb37c45c1eddc6a7dae57de498aae6d5e3589e362d4b4323a533").unwrap()
                    ],
                    data: vec![
                        Felt::from_hex("0x43abaa073c768ebf039c0c4f46db9acc39e9ec165690418060a652aab39e7d8").unwrap(),
                        Felt::from_hex("0x43abaa073c768ebf039c0c4f46db9acc39e9ec165690418060a652aab39e7d8").unwrap(),
                    ],
                    block_hash: Some(
                        Felt::from_hex("0x5c627d4aeb51280058bed93c7889bce78114d63baad1be0f0aeb32496d5f19c").unwrap(),
                    ),
                    block_number: Some(0),
                    transaction_hash: Felt::from_hex(
                        "0x1bec64a9f5ff52154b560fd489ae2aabbfcb31062f7ea70c3c674ddf14b0add",
                    )
                    .unwrap(),
                    event_index: 0,
                    transaction_index: 0,
                },
                EmittedEvent {
                    from_address: Felt::from_hex("0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7")
                        .unwrap(),
                    keys: vec![
                        Felt::from_hex("0x4595132f9b33b7077ebf2e7f3eb746a8e0a6d5c337c71cd8f9bf46cac3cfd7").unwrap()
                    ],
                    data: vec![
                        Felt::from_hex("0x43abaa073c768ebf039c0c4f46db9acc39e9ec165690418060a652aab39e7d8").unwrap()
                    ],
                    block_hash: Some(
                        Felt::from_hex("0x5c627d4aeb51280058bed93c7889bce78114d63baad1be0f0aeb32496d5f19c").unwrap(),
                    ),
                    block_number: Some(0),
                    transaction_hash: Felt::from_hex(
                        "0x1bec64a9f5ff52154b560fd489ae2aabbfcb31062f7ea70c3c674ddf14b0add",
                    )
                    .unwrap(),
                    event_index: 0,
                    transaction_index: 0,
                },
            ],
            continuation_token: Some("4-0".to_string()),
        };

        assert_eq!(events.events, expected_events.events);
        assert_type_equality(&events.continuation_token, &expected_events.continuation_token);
    }

    /// Retrieves events based on a filter with a continuation token.
    ///
    /// Example curl command:
    ///
    /// ```bash
    /// curl --location 'https://free-rpc.nethermind.io/sepolia-juno/' \
    /// --header 'Content-Type: application/json' \
    /// --data '{
    ///     "jsonrpc": "2.0",
    ///     "method": "starknet_getEvents",
    ///     "params": {
    ///         "filter": {
    ///             "from_block": {
    ///                 "block_number": 0
    ///             },
    ///             "to_block": {
    ///                 "block_number": 19
    ///             },
    ///             "address": "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7",
    ///             "keys": [
    ///                 []
    ///             ],
    ///             "continuation_token": "0-2",
    ///             "chunk_size": 2
    ///         }
    ///     },
    ///     "id": 1
    /// }'
    /// ```
    #[rstest]
    #[tokio::test]
    async fn test_get_events_with_continuation_token_works() {
        let madara = get_madara().await;
        let json_client = madara.json_rpc();
        let events = {
            json_client
                .get_events(
                    EventFilter {
                        from_block: Some(BlockId::Number(0)),
                        to_block: Some(BlockId::Number(19)),
                        address: Some(
                            Felt::from_hex("0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7")
                                .unwrap(),
                        ),
                        keys: Some(vec![vec![]]),
                    },
                    Some("0-2".to_string()),
                    2,
                )
                .await
                .unwrap()
        };

        let expected_events = EventsPage {
            events: vec![
                EmittedEvent {
                    from_address: Felt::from_hex("0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7")
                        .unwrap(),
                    keys: vec![
                        Felt::from_hex("0x2e8a4ec40a36a027111fafdb6a46746ff1b0125d5067fbaebd8b5f227185a1e").unwrap()
                    ],
                    data: vec![
                        Felt::from_hex("0x23be95f90bf41685e18a4356e57b0cfdc1da22bf382ead8b64108353915c1e5").unwrap(),
                        Felt::from_hex("0x0").unwrap(),
                        Felt::from_hex("0x4").unwrap(),
                        Felt::from_hex("0x4574686572").unwrap(),
                        Felt::from_hex("0x455448").unwrap(),
                        Felt::from_hex("0x12").unwrap(),
                        Felt::from_hex("0x4c5772d1914fe6ce891b64eb35bf3522aeae1315647314aac58b01137607f3f").unwrap(),
                        Felt::from_hex("0x0").unwrap(),
                    ],
                    block_hash: Some(
                        Felt::from_hex("0x445152a69e628774b0f78a952e6f9ba0ffcda1374724b314140928fd2f31f4c").unwrap(),
                    ),
                    block_number: Some(4),
                    transaction_hash: Felt::from_hex(
                        "0x3c9dfcd3fe66be18b661ee4ebb62520bb4f13d4182b040b3c2be9a12dbcc09b",
                    )
                    .unwrap(),
                    event_index: 0,
                    transaction_index: 0,
                },
                EmittedEvent {
                    from_address: Felt::from_hex("0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7")
                        .unwrap(),
                    keys: vec![
                        Felt::from_hex("0x1205ec81562fc65c367136bd2fe1c0fff2d1986f70e4ba365e5dd747bd08753").unwrap()
                    ],
                    data: vec![
                        Felt::from_hex("0x23be95f90bf41685e18a4356e57b0cfdc1da22bf382ead8b64108353915c1e5").unwrap(),
                        Felt::from_hex("0x0").unwrap(),
                        Felt::from_hex("0x4").unwrap(),
                        Felt::from_hex("0x4574686572").unwrap(),
                        Felt::from_hex("0x455448").unwrap(),
                        Felt::from_hex("0x12").unwrap(),
                        Felt::from_hex("0x4c5772d1914fe6ce891b64eb35bf3522aeae1315647314aac58b01137607f3f").unwrap(),
                    ],
                    block_hash: Some(
                        Felt::from_hex("0x445152a69e628774b0f78a952e6f9ba0ffcda1374724b314140928fd2f31f4c").unwrap(),
                    ),
                    block_number: Some(4),
                    transaction_hash: Felt::from_hex(
                        "0x24ae8900d238a120e927d6a5f2e4ddf85419e97020d92c1fadc615bff666ab1",
                    )
                    .unwrap(),
                    event_index: 0,
                    transaction_index: 0,
                },
            ],
            continuation_token: Some("6-0".to_string()),
        };

        assert_eq!(events.events, expected_events.events);
        assert_type_equality(&events.continuation_token, &expected_events.continuation_token);
    }

    /// Calls a contract function at a specific block.
    ///
    /// Example curl command:
    ///
    /// ```bash
    /// curl --location 'https://free-rpc.nethermind.io/sepolia-juno/' \
    /// --header 'Content-Type: application/json' \
    /// --data '{
    ///     "jsonrpc": "2.0",
    ///     "method": "starknet_call",
    ///     "params": {
    ///         "request": {
    ///             "contract_address": "0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7",
    ///             "entry_point_selector": "0x361458367e696363fbcc70777d07ebbd2394e89fd0adcaf147faccd1d294d60",
    ///             "calldata": []
    ///         },
    ///         "block_id": {
    ///             "block_number": 19
    ///         }
    ///     },
    ///     "id": 1
    /// }'
    /// ```
    ///
    /// Note: The test is ignored as of now because madara support the transaction with starknet version >= 0.13.0
    /// and ideally for the node will call the sequencer to get the result of the transaction for older starknet version.
    #[ignore]
    #[rstest]
    #[tokio::test]
    async fn test_call_works() {
        let madara = get_madara().await;
        let json_client = madara.json_rpc();
        let call_response = {
            json_client
                .call(
                    FunctionCall {
                        contract_address: Felt::from_hex(
                            "0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7",
                        )
                        .unwrap(),
                        entry_point_selector: Felt::from_hex(
                            "0x361458367e696363fbcc70777d07ebbd2394e89fd0adcaf147faccd1d294d60",
                        )
                        .unwrap(),
                        calldata: vec![],
                    },
                    BlockId::Number(19),
                )
                .await
                .unwrap()
        };

        let expected_call_response = vec![Felt::from_hex("0x4574686572").unwrap()];

        assert_eq!(call_response, expected_call_response);
    }

    /// Note: In this test case we are comparing the decompressed program output that we get from this js script
    /// with the decompressed program we get from the madara response.
    ///
    /// Along with that we are also checking for abi and the entry points.
    ///
    /// The js script used to get the decompressed program is:
    ///
    /// ```javascript
    ///     const starknet = require('starknet');
    ///     const contract = require('./juno.json');
    ///
    ///     async function main() {
    ///         let contract_class_parsed = starknet.stark.decompressProgram(
    ///             contract.program
    ///         );
    ///         console.log((contract_class_parsed));
    ///     }
    ///
    ///     main();
    /// ```
    ///
    /// In the above code, juno.json can be obtained by:
    ///
    /// Retrieves the contract class for a given class hash.
    ///
    /// Example curl command:
    ///
    /// ```bash
    /// curl --location 'https://free-rpc.nethermind.io/sepolia-juno/' \
    /// --header 'Content-Type: application/json' \
    /// --data '{
    ///     "jsonrpc": "2.0",
    ///     "method": "starknet_getClass",
    ///     "params": {
    ///         "block_id": {
    ///             "block_number": 12
    ///         },
    ///         "class_hash": "0x05c478ee27f2112411f86f207605b2e2c58cdb647bac0df27f660ef2252359c6"
    ///     },
    ///     "id": 1
    /// }'
    /// ```
    ///
    /// Note: In this test case we are comparing the decompressed program output that we get from this js script
    /// with the decompressed program we get from the madara response.
    ///
    /// Along with that we are also checking for abi and the entry points.
    #[rstest]
    #[tokio::test]
    async fn test_get_class_works() {
        let madara = get_madara().await;
        let json_client = madara.json_rpc();
        let contract_class = {
            json_client
                .get_class(
                    BlockId::Number(12),
                    Felt::from_hex("0x05c478ee27f2112411f86f207605b2e2c58cdb647bac0df27f660ef2252359c6").unwrap(),
                )
                .await
                .unwrap()
        };

        compare_contract_class(contract_class);
    }

    /// Retrieves the compiled contract class for a given class hash.
    /// The class must be a Sierra contract class.
    ///
    /// Example curl command:
    ///
    /// ```bash
    /// curl --location 'https://free-rpc.nethermind.io/sepolia-juno/' \
    /// --header 'Content-Type: application/json' \
    /// --data '{
    ///     "jsonrpc": "2.0",
    ///     "method": "starknet_getCompiledCasm",
    ///     "params": {
    ///         "class_hash": "0x903752516de5c04fe91600ca6891e325278b2dfc54880ae11a809abb364844"
    ///     },
    ///     "id": 1
    /// }'
    /// ```
    ///
    /// Note: The `compiler_version` field is ignored in the comparison because,
    /// for Sierra class versions > v1.1.0, we always use the latest version of the compiler.
    #[rstest]
    #[tokio::test]
    async fn test_get_compiled_casm_works() {
        let madara = get_madara().await;

        // use a reqwest client because starknet-providers does not support the rpc call yet
        let client = reqwest::Client::new();

        // use the v0.8.0 rpc endpoint
        let url = madara.rpc_url.clone().unwrap().join("rpc/v0_8_1/").unwrap();

        let res = client
            .post(url)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "starknet_getCompiledCasm",
                "params": {
                    "class_hash": "0x903752516de5c04fe91600ca6891e325278b2dfc54880ae11a809abb364844",
                },
                "id": 0
            }))
            .send()
            .await
            .unwrap();

        let mut response = res.json::<serde_json::Value>().await.unwrap();
        let result = response.get_mut("result").unwrap();
        result.as_object_mut().unwrap().remove("compiler_version");

        let mut expected_compiled_class: serde_json::Value =
            serde_json::from_slice(include_bytes!("test_utils/compiled_class.json")).unwrap();
        expected_compiled_class.as_object_mut().unwrap().remove("compiler_version");

        assert_eq!(result, &expected_compiled_class);
    }

    /// Retrieves the contract class at a specific address.
    ///
    /// Example curl command:
    ///
    /// ```bash
    /// curl --location 'https://free-rpc.nethermind.io/sepolia-juno/' \
    /// --header 'Content-Type: application/json' \
    /// --data '{
    ///     "jsonrpc": "2.0",
    ///     "method": "starknet_getClassAt",
    ///     "params": {
    ///         "block_id": {
    ///             "block_number": 12
    ///         },
    ///         "contract_address": "0x043abaa073c768ebf039c0c4f46db9acc39e9ec165690418060a652aab39e7d8"
    ///     },
    ///     "id": 1
    /// }'
    /// ```
    ///
    /// Note: The program has been compressed using the same script mentioned in the above test case.
    #[rstest]
    #[tokio::test]
    async fn test_get_class_at_works() {
        let madara = get_madara().await;
        let json_client = madara.json_rpc();
        let contract_class = {
            json_client
                .get_class_at(
                    BlockId::Number(12),
                    Felt::from_hex("0x043abaa073c768ebf039c0c4f46db9acc39e9ec165690418060a652aab39e7d8").unwrap(),
                )
                .await
                .unwrap()
        };

        compare_contract_class(contract_class);
    }

    /// Estimates the fee for a transaction or a list of transactions.
    ///
    /// Example curl command:
    ///
    /// ```bash
    /// curl --location 'https://free-rpc.nethermind.io/sepolia-juno/' \
    /// --header 'Content-Type: application/json' \
    /// --data '{
    ///     "jsonrpc": "2.0",
    ///     "method": "starknet_estimateFee",
    ///     "params": {
    ///         "request": [
    ///             {
    ///                 "type": "DEPLOY_ACCOUNT",
    ///                 "version": "0x1",
    ///                 "max_fee": "0x0",
    ///                 "signature": [
    ///                     "0x73d0a8a69f0ebf44b1c2bb2a9e85bf998883eb2008ca7b9c57b6f28dacb6dd8",
    ///                     "0x4a43711cd08f55ef73603f1e7b880c7f438fb68934f0823a736f9f577ab040a",
    ///                     "0x0",
    ///                     "0x0",
    ///                     "0x0",
    ///                     "0x0",
    ///                     "0x0",
    ///                     "0x0",
    ///                     "0x0",
    ///                     "0x0"
    ///                 ],
    ///                 "nonce": "0x0",
    ///                 "contract_address_salt": "0x0",
    ///                 "constructor_calldata": ["0x2e23f1647b018bfb3fe107e2ebd4412f0a0ed41bd60c10d842a76f8cdbbe1ba"],
    ///                 "class_hash": "0x05c478ee27f2112411f86f207605b2e2c58cdb647bac0df27f660ef2252359c6"
    ///             }
    ///         ],
    ///         "simulation_flags": [ "SKIP_VALIDATE"],
    ///         "block_id": {
    ///             "block_number": 1
    ///         }
    ///     },
    ///     "id": 1
    /// }'
    /// ```
    ///
    /// Note: The test is ignored as of now because madara support the transaction with starknet version >= 0.13.0
    /// and ideally for the node will call the sequencer to get the result of the transaction for older starknet version.
    #[ignore]
    #[rstest]
    #[tokio::test]
    async fn test_estimate_fee_works() {
        let madara = get_madara().await;
        let json_client = madara.json_rpc();
        let call_response = {
            json_client
                .estimate_fee(
                    vec![BroadcastedTransaction::DeployAccount(BroadcastedDeployAccountTransactionV3 {
                        signature: vec![
                            Felt::from_hex("0x73d0a8a69f0ebf44b1c2bb2a9e85bf998883eb2008ca7b9c57b6f28dacb6dd8")
                                .unwrap(),
                            Felt::from_hex("0x4a43711cd08f55ef73603f1e7b880c7f438fb68934f0823a736f9f577ab040a")
                                .unwrap(),
                            Felt::from_hex("0x0").unwrap(),
                            Felt::from_hex("0x0").unwrap(),
                            Felt::from_hex("0x0").unwrap(),
                            Felt::from_hex("0x0").unwrap(),
                            Felt::from_hex("0x0").unwrap(),
                            Felt::from_hex("0x0").unwrap(),
                            Felt::from_hex("0x0").unwrap(),
                            Felt::from_hex("0x0").unwrap(),
                        ],
                        nonce: Felt::from_hex("0x0").unwrap(),
                        contract_address_salt: Felt::from_hex("0x0").unwrap(),
                        constructor_calldata: vec![Felt::from_hex(
                            "0x2e23f1647b018bfb3fe107e2ebd4412f0a0ed41bd60c10d842a76f8cdbbe1ba",
                        )
                        .unwrap()],
                        class_hash: Felt::from_hex(
                            "0x05c478ee27f2112411f86f207605b2e2c58cdb647bac0df27f660ef2252359c6",
                        )
                        .unwrap(),
                        is_query: true,
                        resource_bounds: ResourceBoundsMapping {
                            l1_gas: ResourceBounds { max_amount: 10000, max_price_per_unit: 10000 },
                            l1_data_gas: ResourceBounds { max_amount: 10000, max_price_per_unit: 10000 },
                            l2_gas: ResourceBounds { max_amount: 10000, max_price_per_unit: 10000 },
                        },
                        tip: Default::default(),
                        paymaster_data: Default::default(),
                        nonce_data_availability_mode: DataAvailabilityMode::L1,
                        fee_data_availability_mode: DataAvailabilityMode::L1,
                    })],
                    vec![SimulationFlagForEstimateFee::SkipValidate],
                    BlockId::Number(1),
                )
                .await
                .unwrap()
        };

        let expected_call_response: Vec<FeeEstimate> = vec![FeeEstimate {
            l1_gas_consumed: 0,
            l1_gas_price: 0,
            l2_gas_consumed: 0,
            l2_gas_price: 0,
            l1_data_gas_consumed: 0,
            l1_data_gas_price: 0,
            overall_fee: 0,
        }];
        assert_eq!(call_response, expected_call_response);
    }

    /// Estimates the fee for a message from L1 to L2.
    ///
    /// Example curl command:
    ///
    /// ```bash
    /// curl --location 'https://free-rpc.nethermind.io/sepolia-juno/' \
    /// --header 'Content-Type: application/json' \
    /// --data '{
    ///     "jsonrpc": "2.0",
    ///     "method": "starknet_estimateMessageFee",
    ///     "params": {
    ///         "message": {
    ///             "from_address": "0x8453fc6cd1bcfe8d4dfc069c400b433054d47bdc",
    ///             "to_address": "0x04c5772d1914fe6ce891b64eb35bf3522aeae1315647314aac58b01137607f3f",
    ///             "entry_point_selector": "0x2d757788a8d8d6f21d1cd40bce38a8222d70654214e96ff95d8086e684fbee5",
    ///             "payload": ["755459732925565255818828693099006464755347258332","0","0"]
    ///         },
    ///         "block_id": {
    ///             "block_number": 8
    ///         }
    ///     },
    ///     "id": 1
    /// }'
    /// ```
    ///
    /// Note: The test is ignored as of now because madara support the transaction with starknet version >= 0.13.0
    /// and ideally for the node will call the sequencer to get the result of the transaction for older starknet version.
    #[ignore]
    #[rstest]
    #[tokio::test]
    async fn test_estimate_message_fee_works() {
        let madara = get_madara().await;
        let json_client = madara.json_rpc();
        let call_response = {
            json_client
                .estimate_message_fee(
                    MsgFromL1 {
                        from_address: EthAddress::from_hex("0x8453fc6cd1bcfe8d4dfc069c400b433054d47bdc").unwrap(),
                        to_address: Felt::from_hex(
                            "0x04c5772d1914fe6ce891b64eb35bf3522aeae1315647314aac58b01137607f3f",
                        )
                        .unwrap(),
                        entry_point_selector: Felt::from_hex(
                            "0x2d757788a8d8d6f21d1cd40bce38a8222d70654214e96ff95d8086e684fbee5",
                        )
                        .unwrap(),
                        payload: vec![
                            Felt::from_hex("0x8453fc6cd1bcfe8d4dfc069c400b433054d47bdc").unwrap(),
                            Felt::from_hex("0x0").unwrap(),
                            Felt::from_hex("0x0").unwrap(),
                        ],
                    },
                    BlockId::Number(8),
                )
                .await
                .unwrap()
        };

        let expected_call_response: MessageFeeEstimate = MessageFeeEstimate {
            l1_gas_consumed: 0,
            l1_gas_price: 0,
            l2_gas_consumed: 0,
            l2_gas_price: 0,
            l1_data_gas_consumed: 0,
            l1_data_gas_price: 0,
            overall_fee: 0,
        };
        assert_eq!(call_response, expected_call_response);
    }

    fn compare_contract_class(contract_class: ContractClass) {
        let contract_program = match contract_class.clone() {
            ContractClass::Legacy(compressed) => Some(compressed.program),
            _ => None,
        };

        let contract_entry_points = match contract_class.clone() {
            ContractClass::Legacy(compressed) => Some(compressed.entry_points_by_type),
            _ => None,
        };

        let contract_abi = match contract_class.clone() {
            ContractClass::Legacy(compressed) => Some(compressed.abi),
            _ => None,
        };

        let decompressed_program = decompress_to_string(contract_program.unwrap());

        let expected_program = include_str!("test_utils/class_program.txt");
        let expected_contract_class: ContractClass =
            serde_json::from_slice(include_bytes!("test_utils/contract_class.json")).unwrap();

        let expected_contract_entry_points = match expected_contract_class.clone() {
            ContractClass::Legacy(compressed) => Some(compressed.entry_points_by_type),
            _ => None,
        };

        let expected_contract_abi = match expected_contract_class.clone() {
            ContractClass::Legacy(compressed) => Some(compressed.abi),
            _ => None,
        };

        assert_eq!(decompressed_program, expected_program);
        assert_eq!(contract_entry_points, expected_contract_entry_points);
        assert_eq!(contract_abi, expected_contract_abi);
    }

    fn decompress_to_string(data: Vec<u8>) -> String {
        let mut decoder = GzDecoder::new(&data[..]);
        let mut decompressed = String::new();

        match decoder.read_to_string(&mut decompressed) {
            Ok(_) => decompressed,
            Err(_e) => {
                // If decompression fails, try to interpret as UTF-8
                match String::from_utf8(data.clone()) {
                    Ok(s) => s,
                    Err(_) => {
                        // If both fail, return the data as a hexadecimal string
                        format!(
                            "Error: Unable to decompress or decode data. Hexadecimal representation: {}",
                            data.iter().fold(String::new(), |mut acc, b| {
                                write!(&mut acc, "{:02x}", b).unwrap();
                                acc
                            })
                        )
                    }
                }
            }
        }
    }

    fn assert_type_equality<T: Any, U: Any>(_t: &T, _u: &U) {
        assert_eq!(
            std::any::TypeId::of::<T>(),
            std::any::TypeId::of::<U>(),
            "Types are not equal: {:?} != {:?}",
            std::any::TypeId::of::<T>(),
            std::any::TypeId::of::<U>()
        );
    }
}
