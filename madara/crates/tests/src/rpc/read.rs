// source of truth for the data here is the juno rpc. for each method tested here, same call was
// made to the juno rpc.

#[cfg(test)]
mod test_rpc_read_calls {
    use crate::rpc::utils::{RpcTestContext, DEVNET_ACCOUNTS, ETH_CONTRACT_ADDRESS, MAX_FEE, STRK_CONTRACT_ADDRESS};
    use blockifier::abi::abi_utils::get_fee_token_var_address;
    use flate2::read::GzDecoder;
    use rstest::rstest;
    use starknet::macros::{felt, selector};
    use starknet_core::types::{
        BlockHashAndNumber, BlockId, BlockStatus, BlockWithReceipts, BlockWithTxHashes, BlockWithTxs,
        BroadcastedInvokeTransaction, BroadcastedInvokeTransactionV1, ComputationResources, ContractClass,
        ContractStorageDiffItem, DataAvailabilityResources, DataResources, DeclareTransaction,
        DeclareTransactionReceipt, DeclareTransactionV0, EmittedEvent, Event, EventFilter, EventsPage,
        ExecutionResources, ExecutionResult, FeePayment, FunctionCall, InvokeTransaction, InvokeTransactionReceipt,
        InvokeTransactionV1, L1DataAvailabilityMode, MaybePendingBlockWithReceipts, MaybePendingBlockWithTxHashes,
        MaybePendingBlockWithTxs, MaybePendingStateUpdate, NonceUpdate, PriceUnit, ReceiptBlock, ResourcePrice,
        StarknetError, StateDiff, StateUpdate, StorageEntry, Transaction, TransactionExecutionStatus,
        TransactionFinalityStatus, TransactionReceipt, TransactionReceiptWithBlockInfo, TransactionStatus,
        TransactionWithReceipt,
    };
    use starknet_core::types::{
        BroadcastedDeployAccountTransaction, BroadcastedDeployAccountTransactionV1, BroadcastedTransaction, EthAddress,
        FeeEstimate, MsgFromL1, SimulationFlagForEstimateFee,
    };
    use starknet_providers::{Provider, ProviderError};
    use std::any::Any;
    use std::fmt::Write;
    use std::io::Read;
    use tokio::sync::OnceCell;

    static TEST_CONTEXT: OnceCell<RpcTestContext> = OnceCell::const_new();

    pub async fn get_test_context() -> &'static RpcTestContext {
        TEST_CONTEXT
            .get_or_init(|| async {
                let mut ctx = RpcTestContext::new().await;
                ctx.block_1().await;
                ctx.madara.wait_for_sync_to(1).await;
                ctx.block_2().await;
                ctx.madara.wait_for_sync_to(2).await;
                ctx
            })
            .await
    }

    /// Fetches the latest block hash and number.
    #[rstest]
    #[tokio::test]
    async fn test_block_hash_and_number_works() {
        let client = get_test_context().await.rpc_client();

        let result = { client.block_hash_and_number().await.unwrap() };
        assert_eq!(
            result,
            BlockHashAndNumber {
                block_hash: felt!("0x351eb09995bec11a120aa85cae65d02052e9a19fe4e17c864c65d224aa0fdf1"),
                block_number: 2
            }
        );
    }

    /// Retrieves the number of transactions in a specific block.
    #[rstest]
    #[tokio::test]
    async fn test_get_block_txn_count_works() {
        let client = get_test_context().await.rpc_client();
        let result = { client.get_block_transaction_count(BlockId::Number(1)).await.unwrap() };
        assert_eq!(result, 10);
    }

    #[rstest]
    #[tokio::test]
    async fn test_batched_requests_work() {
        let url = get_test_context().await.madara.rpc_url.clone();

        // use reqwest to send a batch request to the madara rpc.
        // TODO: use a jsonrpc client instead of reqwest when we move
        // to starknet-providers 0.12.0
        let client = reqwest::Client::new();
        let res = client
            .post(url)
            .json(&[
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "starknet_getBlockTransactionCount",
                    "params": {
                        "block_id": {
                            "block_number": 1
                        }
                    },
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
                "result": 10,
                "id": 0
            })
        );
        assert_eq!(
            result[1],
            serde_json::json!({
                "jsonrpc": "2.0",
                "result": 3,
                "id": 1
            })
        );
    }

    // TODO: fix by updating the rpc client
    /// Fetches a block with its transactions and receipts.
    #[ignore]
    #[rstest]
    #[tokio::test]
    async fn test_get_block_txn_with_receipts_works() {
        let client = get_test_context().await.rpc_client();

        let block = client
            .get_block_with_receipts(BlockId::Number(2))
            .await
            .expect("Failed to get block with receipts for block number 2");

        let expected_block = MaybePendingBlockWithReceipts::Block(BlockWithReceipts {
            status: BlockStatus::AcceptedOnL2,
            block_hash: felt!("0x7a906dfd1ff77a121b8048e6f750cda9e949d341c4487d4c6a449f183f0e61d"),
            parent_hash: felt!("0x78b67b11f8c23850041e11fb0f3b39db0bcb2c99d756d5a81321d1b483d79f6"),
            block_number: 2,
            new_root: felt!("0xe005205a1327f3dff98074e528f7b96f30e0624a1dfcf571bdc81948d150a0"),
            timestamp: 1700475581,
            sequencer_address: felt!("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8"),
            l1_gas_price: ResourcePrice { price_in_fri: felt!("0x0"), price_in_wei: felt!("0x3b9ad016") },
            l1_data_gas_price: ResourcePrice { price_in_fri: felt!("0x1"), price_in_wei: felt!("0x1") },
            l1_da_mode: L1DataAvailabilityMode::Calldata,
            starknet_version: "0.12.3".to_string(),
            transactions: vec![TransactionWithReceipt {
                transaction: Transaction::Declare(DeclareTransaction::V0(DeclareTransactionV0 {
                    transaction_hash: felt!("0x701d9adb9c60bc2fd837fe3989e15aeba4be1a6e72bb6f61ffe35a42866c772"),
                    sender_address: felt!("0x1"),
                    max_fee: felt!("0x0"),
                    signature: vec![],
                    class_hash: felt!("0x4f23a756b221f8ce46b72e6a6b10ee7ee6cf3b59790e76e02433104f9a8c5d1"),
                })),
                receipt: {
                    TransactionReceipt::Declare(DeclareTransactionReceipt {
                        transaction_hash: felt!("0x701d9adb9c60bc2fd837fe3989e15aeba4be1a6e72bb6f61ffe35a42866c772"),
                        actual_fee: FeePayment { amount: felt!("0x0"), unit: PriceUnit::Wei },
                        finality_status: TransactionFinalityStatus::AcceptedOnL2,
                        messages_sent: vec![],
                        events: vec![],
                        execution_resources: ExecutionResources {
                            computation_resources: ComputationResources {
                                steps: 2711,
                                memory_holes: None,
                                range_check_builtin_applications: Some(63),
                                pedersen_builtin_applications: Some(15),
                                poseidon_builtin_applications: None,
                                ec_op_builtin_applications: None,
                                ecdsa_builtin_applications: None,
                                bitwise_builtin_applications: None,
                                keccak_builtin_applications: None,
                                segment_arena_builtin: None,
                            },
                            data_resources: DataResources {
                                data_availability: DataAvailabilityResources { l1_gas: 0, l1_data_gas: 0 },
                            },
                        },
                        execution_result: ExecutionResult::Succeeded,
                    })
                },
            }],
        });
        assert_eq!(block, expected_block);
    }

    /// Retrieves a block with transaction hashes.
    #[rstest]
    #[tokio::test]
    async fn test_get_block_txn_with_tx_hashes_works() {
        let client = get_test_context().await.rpc_client();

        let block = { client.get_block_with_tx_hashes(BlockId::Number(1)).await.unwrap() };

        let expected_block = MaybePendingBlockWithTxHashes::Block(BlockWithTxHashes {
            status: BlockStatus::AcceptedOnL2,
            block_hash: felt!("0x7fec16502476dad04e30fd8042942441af53b58b7142ac639017fd959c9879f"),
            parent_hash: felt!("0x1ebfc63d8c573df33434ddfb0f7c2d67eb0e6ec25b2d2903efe5233be5b3347"),
            block_number: 1,
            new_root: felt!("0x1c9caa6ebf140623100d04ad0c923fc59824921e022a501756cefe9d0c4ba85"),
            timestamp: 0,
            sequencer_address: felt!("0x123"),
            l1_gas_price: ResourcePrice { price_in_fri: felt!("0x0"), price_in_wei: felt!("0xa") },
            l1_data_gas_price: ResourcePrice { price_in_fri: felt!("0x0"), price_in_wei: felt!("0x0") },
            l1_da_mode: L1DataAvailabilityMode::Blob,
            starknet_version: "0.13.2".to_string(),
            transactions: vec![
                felt!("0x7fa75444e0a4138c927b0c1a65b6a14959ddc3f592369a9381674a2bbbbbae1"),
                felt!("0x5c49e4eb18cf49d76e3b653b82d8662b93640bb70b019b165e5a733c19e2657"),
                felt!("0x6042a5eac0ed230bb85fa7854ab96f3172ef86a43e50eae9cbe0ecfc1dddc23"),
                felt!("0x47adaedd07e0cb1f37d6d578a041ec9da336f0eb29df8b123e2c17759afe9a6"),
                felt!("0x275e6c0748c5d6bf884218cdd48cdc9d965c4797cf6b87885b7280259b4b75"),
                felt!("0x762b30e6b525402b5e3a2a3ec5ce7a1a4532ae26860b70ac1af69dbe1a9bf8f"),
                felt!("0x501840aa5e432ebf35036198582f40431d5b5d114ee8159f7f20baa94fcc70"),
                felt!("0x6ab5b31161f5d67be21633baa636f70c1a5a6fa86a4c3156a960822d21c06ab"),
                felt!("0x4df672fe1d5e5d7d09edf4a4d4d3817fa932be751387713b3ab59da0d16ebe0"),
                felt!("0x1171974ecdb1074a2f616f740f39a280e66ccfebbd810c76d6b93fe149258d0"),
            ],
        });
        assert_eq!(block, expected_block);
    }

    #[rstest]
    #[tokio::test]
    async fn test_get_block_txn_with_tx_hashes_fails_block_not_exists() {
        let client = get_test_context().await.rpc_client();

        let block = client.get_block_with_tx_hashes(BlockId::Number(4)).await;
        assert!(matches!(block, Err(ProviderError::StarknetError(StarknetError::BlockNotFound))));
    }

    /// Fetches a block with full transaction details.
    #[rstest]
    #[tokio::test]
    async fn test_get_block_txn_with_tx_works() {
        let client = get_test_context().await.rpc_client();

        let block = client.get_block_with_txs(BlockId::Number(2)).await.unwrap();

        let expected_block = MaybePendingBlockWithTxs::Block(BlockWithTxs {
            status: BlockStatus::AcceptedOnL2,
            block_hash: felt!("0x351eb09995bec11a120aa85cae65d02052e9a19fe4e17c864c65d224aa0fdf1"),
            parent_hash: felt!("0x7fec16502476dad04e30fd8042942441af53b58b7142ac639017fd959c9879f"),
            block_number: 2,
            new_root: felt!("0x4f6413432d5dfc251371f5ed84e23e5a1bbaa08e3b2cc4f1694568af26dc954"),
            timestamp: 0,
            sequencer_address: felt!("0x123"),
            l1_gas_price: ResourcePrice { price_in_fri: felt!("0x0"), price_in_wei: felt!("0xa") },
            l1_data_gas_price: ResourcePrice { price_in_fri: felt!("0x0"), price_in_wei: felt!("0x0") },
            l1_da_mode: L1DataAvailabilityMode::Blob,
            starknet_version: "0.13.2".to_string(),
            transactions: vec![
                Transaction::Invoke(InvokeTransaction::V1(InvokeTransactionV1 {
                    transaction_hash: felt!("0x6b6eb4109ac9d07744ffe376f67cb04a13b7e1d77ef2ac2a6662a6e53d9fb3c"),
                    sender_address: DEVNET_ACCOUNTS[0].0,
                    calldata: vec![
                        felt!("0x1"),
                        ETH_CONTRACT_ADDRESS,
                        selector!("transfer"),
                        felt!("0x03"),
                        DEVNET_ACCOUNTS[1].0,
                        felt!("0xa"),
                        felt!("0x0"),
                    ],
                    max_fee: felt!("0x186a0"),
                    signature: vec![
                        felt!("0x25e3ef7815d3f8fc82d572d3a48b8d7fff574d9cb6b58cab0f9871f91c8d6d5"),
                        felt!("0x5ed550cac383cd158845cc375888569cce5c2e81f54eb26b08dc0aa3e035986"),
                    ],
                    nonce: felt!("0x1"),
                })),
                Transaction::Invoke(InvokeTransaction::V1(InvokeTransactionV1 {
                    transaction_hash: felt!("0x4449f9b0077ce926d35154d6f2d55aa4dc303dc732c38690df6028f55bf9022"),
                    sender_address: DEVNET_ACCOUNTS[1].0,
                    calldata: vec![
                        felt!("0x1"),
                        ETH_CONTRACT_ADDRESS,
                        selector!("transfer"),
                        felt!("0x03"),
                        DEVNET_ACCOUNTS[2].0,
                        felt!("0x14"),
                        felt!("0x0"),
                    ],
                    max_fee: felt!("0x186a0"),
                    signature: vec![
                        felt!("0x1397915b7615f1f5b2424383434ebcdf385cae887d2618b2769bf019fcdd29e"),
                        felt!("0x5b4430e30c018da43ec8bb87efabb7d008732d9e9ffe9d896aca6e36db7465b"),
                    ],
                    nonce: felt!("0x1"),
                })),
                Transaction::Invoke(InvokeTransaction::V1(InvokeTransactionV1 {
                    transaction_hash: felt!("0x67a98b6a9ee79b1c442699dfcaef3512e8ff9eaf7c53a5dff3502bb30c67dc0"),
                    sender_address: DEVNET_ACCOUNTS[2].0,
                    calldata: vec![
                        felt!("0x1"),
                        ETH_CONTRACT_ADDRESS,
                        selector!("transfer"),
                        felt!("0x03"),
                        DEVNET_ACCOUNTS[3].0,
                        felt!("0x1e"),
                        felt!("0x0"),
                    ],
                    max_fee: felt!("0x186a0"),
                    signature: vec![
                        felt!("0x668bde75b3c575ac012c9dc85709d06800cc7068d6d620a8d7f855c2ba14f65"),
                        felt!("0x48c1e76306538867c754c9dc4c70eae397ce6202112965bf97b55863661c370"),
                    ],
                    nonce: felt!("0x1"),
                })),
            ],
        });
        assert_eq!(block, expected_block);
    }

    #[rstest]
    #[tokio::test]
    async fn test_get_block_txn_with_tx_fails_block_not_exists() {
        let client = get_test_context().await.rpc_client();

        let block = client.get_block_with_txs(BlockId::Number(4)).await;
        assert!(matches!(block, Err(ProviderError::StarknetError(StarknetError::BlockNotFound))));
    }

    /// Retrieves the class hash at a specific block and contract address.
    #[rstest]
    #[tokio::test]
    async fn test_get_class_hash_at_works() {
        let client = get_test_context().await.rpc_client();

        let class_hash = { client.get_class_hash_at(BlockId::Number(2), STRK_CONTRACT_ADDRESS).await.unwrap() };
        let expected_class_hash = felt!("0x40b9e69e14ddc34a98ec8133c80807c144b818bc6cbf5a119d8f62535258142");

        assert_eq!(class_hash, expected_class_hash);
    }

    #[rstest]
    #[tokio::test]
    async fn test_get_class_hash_at_contract_not_exists() {
        let client = get_test_context().await.rpc_client();

        let class_hash = client.get_class_hash_at(BlockId::Number(2), felt!("0x123")).await;
        assert!(matches!(class_hash, Err(ProviderError::StarknetError(StarknetError::ContractNotFound))));
    }

    #[rstest]
    #[tokio::test]
    async fn test_get_class_hash_at_contract_block_not_exists() {
        let client = get_test_context().await.rpc_client();

        let class_hash = client.get_class_hash_at(BlockId::Number(4), STRK_CONTRACT_ADDRESS).await;
        assert!(matches!(class_hash, Err(ProviderError::StarknetError(StarknetError::BlockNotFound))));
    }

    /// Retrieves the nonce for a specific contract address at a given block.
    #[rstest]
    #[tokio::test]
    async fn test_get_nonce_works() {
        let client = get_test_context().await.rpc_client();

        let nonce = { client.get_nonce(BlockId::Number(2), DEVNET_ACCOUNTS[0].0).await.unwrap() };
        let expected_nonce = felt!("0x2");

        assert_eq!(nonce, expected_nonce);
    }

    #[rstest]
    #[tokio::test]
    async fn test_get_nonce_account_not_exists() {
        let client = get_test_context().await.rpc_client();

        let nonce = client.get_nonce(BlockId::Number(2), felt!("0x123")).await;
        assert!(matches!(nonce, Err(ProviderError::StarknetError(StarknetError::ContractNotFound))));
    }

    #[rstest]
    #[tokio::test]
    async fn test_get_nonce_block_not_exists() {
        let client = get_test_context().await.rpc_client();

        let nonce = client.get_nonce(BlockId::Number(4), DEVNET_ACCOUNTS[0].0).await;
        assert!(matches!(nonce, Err(ProviderError::StarknetError(StarknetError::BlockNotFound))));
    }

    /// Retrieves a transaction by its block ID and index.
    #[rstest]
    #[tokio::test]
    async fn test_get_txn_by_block_id_and_index_works() {
        let client = get_test_context().await.rpc_client();

        let txn = { client.get_transaction_by_block_id_and_index(BlockId::Number(1), 4).await.unwrap() };
        let expected_txn = Transaction::Invoke(InvokeTransaction::V1(InvokeTransactionV1 {
            transaction_hash: felt!("0x275e6c0748c5d6bf884218cdd48cdc9d965c4797cf6b87885b7280259b4b75"),
            sender_address: DEVNET_ACCOUNTS[4].0,
            calldata: vec![
                felt!("0x1"),
                STRK_CONTRACT_ADDRESS,
                selector!("transfer"),
                felt!("0x03"),
                DEVNET_ACCOUNTS[5].0,
                felt!("0x3c"),
                felt!("0x0"),
            ],
            max_fee: MAX_FEE,
            signature: vec![
                felt!("0xbc1dbf9e3cd2c054c4c59cca9d4b4b52bc0f28c16d855cae56511652cfb826"),
                felt!("0x18ce34535c6770b3f2691dd0f2bca0e9ad1b11858f9a9d0864106f2359f7c9e"),
            ],
            nonce: felt!("0x0"),
        }));

        assert_eq!(txn, expected_txn);
    }

    #[rstest]
    #[tokio::test]
    async fn test_get_txn_by_block_id_and_index_failes_block_not_exists() {
        let client = get_test_context().await.rpc_client();

        let txn = client.get_transaction_by_block_id_and_index(BlockId::Number(4), 0).await;
        assert!(matches!(txn, Err(ProviderError::StarknetError(StarknetError::BlockNotFound))));
    }

    #[rstest]
    #[tokio::test]
    async fn test_get_txn_by_block_id_and_index_fails_txn_id_out_of_range() {
        let client = get_test_context().await.rpc_client();

        let txn = client.get_transaction_by_block_id_and_index(BlockId::Number(1), 10).await;
        assert!(matches!(txn, Err(ProviderError::StarknetError(StarknetError::InvalidTransactionIndex))));
    }

    /// Retrieves a transaction by its hash.
    #[rstest]
    #[tokio::test]
    async fn test_get_txn_by_hash_works() {
        let client = get_test_context().await.rpc_client();

        let txn = {
            client
                .get_transaction_by_hash(felt!("0x275e6c0748c5d6bf884218cdd48cdc9d965c4797cf6b87885b7280259b4b75"))
                .await
                .unwrap()
        };
        let expected_txn = Transaction::Invoke(InvokeTransaction::V1(InvokeTransactionV1 {
            transaction_hash: felt!("0x275e6c0748c5d6bf884218cdd48cdc9d965c4797cf6b87885b7280259b4b75"),
            sender_address: DEVNET_ACCOUNTS[4].0,
            calldata: vec![
                felt!("0x1"),
                STRK_CONTRACT_ADDRESS,
                selector!("transfer"),
                felt!("0x03"),
                DEVNET_ACCOUNTS[5].0,
                felt!("0x3c"),
                felt!("0x0"),
            ],
            max_fee: MAX_FEE,
            signature: vec![
                felt!("0xbc1dbf9e3cd2c054c4c59cca9d4b4b52bc0f28c16d855cae56511652cfb826"),
                felt!("0x18ce34535c6770b3f2691dd0f2bca0e9ad1b11858f9a9d0864106f2359f7c9e"),
            ],
            nonce: felt!("0x0"),
        }));

        assert_eq!(txn, expected_txn);
    }

    #[rstest]
    #[tokio::test]
    async fn test_get_txn_by_hash_fails_txn_not_exists() {
        let client = get_test_context().await.rpc_client();

        let txn = client.get_transaction_by_hash(felt!("0x123")).await;
        assert!(matches!(txn, Err(ProviderError::StarknetError(StarknetError::TransactionHashNotFound))));
    }

    /// Retrieves the receipt for a transaction.
    #[rstest]
    #[tokio::test]
    // TODO: replace this with jsonrpsee client
    async fn test_get_txn_receipt_works() {
        let client = get_test_context().await.rpc_client();

        let txn_receipt = {
            client
                .get_transaction_receipt(felt!("0x275e6c0748c5d6bf884218cdd48cdc9d965c4797cf6b87885b7280259b4b75"))
                .await
                .unwrap()
        };
        let expected_txn_receipt = TransactionReceiptWithBlockInfo {
            receipt: TransactionReceipt::Invoke(InvokeTransactionReceipt {
                transaction_hash: felt!("0x275e6c0748c5d6bf884218cdd48cdc9d965c4797cf6b87885b7280259b4b75"),
                actual_fee: FeePayment { amount: felt!("0x226"), unit: PriceUnit::Wei },
                finality_status: TransactionFinalityStatus::AcceptedOnL2,
                messages_sent: vec![],
                events: vec![
                    Event {
                        from_address: STRK_CONTRACT_ADDRESS,
                        keys: vec![
                            felt!("0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9"),
                            DEVNET_ACCOUNTS[4].0,
                            DEVNET_ACCOUNTS[5].0,
                        ],
                        data: vec![felt!("0x32"), felt!("0x0")],
                    },
                    Event {
                        from_address: ETH_CONTRACT_ADDRESS,
                        keys: vec![
                            felt!("0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9"),
                            DEVNET_ACCOUNTS[4].0,
                            felt!("0x123"),
                        ],
                        data: vec![felt!("0x226"), felt!("0x0")],
                    },
                ],
                execution_resources: ExecutionResources {
                    computation_resources: ComputationResources {
                        steps: 4382,
                        memory_holes: Some(86),
                        range_check_builtin_applications: Some(139),
                        pedersen_builtin_applications: None,
                        poseidon_builtin_applications: None,
                        ec_op_builtin_applications: None,
                        ecdsa_builtin_applications: None,
                        bitwise_builtin_applications: None,
                        keccak_builtin_applications: None,
                        segment_arena_builtin: None,
                    },
                    data_resources: DataResources {
                        data_availability: DataAvailabilityResources { l1_gas: 0, l1_data_gas: 320 },
                    },
                },
                execution_result: ExecutionResult::Succeeded,
            }),
            block: ReceiptBlock::Block {
                block_hash: felt!("0x7fec16502476dad04e30fd8042942441af53b58b7142ac639017fd959c9879f"),
                block_number: 1,
            },
        };

        assert_eq!(txn_receipt, expected_txn_receipt);
    }

    #[rstest]
    #[tokio::test]
    async fn test_get_txn_receipt_fails_txn_not_exists() {
        let client = get_test_context().await.rpc_client();

        let txn_receipt = client.get_transaction_receipt(felt!("0x123")).await;
        assert!(matches!(txn_receipt, Err(ProviderError::StarknetError(StarknetError::TransactionHashNotFound))));
    }

    /// Retrieves the status of a transaction.
    #[rstest]
    #[tokio::test]
    async fn test_get_txn_status_works_succeded() {
        let client = get_test_context().await.rpc_client();

        let txn_status = {
            client
                .get_transaction_status(felt!("0x275e6c0748c5d6bf884218cdd48cdc9d965c4797cf6b87885b7280259b4b75"))
                .await
                .unwrap()
        };
        let expected_txn_status = TransactionStatus::AcceptedOnL2(TransactionExecutionStatus::Succeeded);

        assert_eq!(txn_status, expected_txn_status);
    }

    #[rstest]
    #[tokio::test]
    async fn test_get_txn_status_works_reverted() {
        let client = get_test_context().await.rpc_client();

        let txn_status = {
            client
                .get_transaction_status(felt!("0x1171974ecdb1074a2f616f740f39a280e66ccfebbd810c76d6b93fe149258d0"))
                .await
                .unwrap()
        };
        let expected_txn_status = TransactionStatus::AcceptedOnL2(TransactionExecutionStatus::Reverted);

        assert_eq!(txn_status, expected_txn_status);
    }

    #[rstest]
    #[tokio::test]
    async fn test_get_txn_status_fails_txn_not_exists() {
        let client = get_test_context().await.rpc_client();

        let txn_status = client.get_transaction_status(felt!("0x123")).await;
        assert!(matches!(txn_status, Err(ProviderError::StarknetError(StarknetError::TransactionHashNotFound))));
    }

    /// Retrieves the storage value at a specific contract address, key, and block.
    #[rstest]
    #[tokio::test]
    async fn test_get_storage_at_works() {
        let client = get_test_context().await.rpc_client();

        let storage_response = {
            client
                .get_storage_at(
                    STRK_CONTRACT_ADDRESS,
                    get_fee_token_var_address(DEVNET_ACCOUNTS[0].0.try_into().unwrap()).0.key(),
                    BlockId::Number(1),
                )
                .await
                .unwrap()
        };
        let expected_storage_response = felt!("0x21e19e0c9bab23ffff6");

        assert_eq!(storage_response, expected_storage_response);
    }

    #[rstest]
    #[tokio::test]
    async fn test_get_storage_at_works_key_not_exists() {
        let client = get_test_context().await.rpc_client();

        let storage_response =
            client.get_storage_at(STRK_CONTRACT_ADDRESS, felt!("0x123"), BlockId::Number(1)).await.unwrap();
        let expected_storage_response = felt!("0x0");

        assert_eq!(storage_response, expected_storage_response);
    }

    #[rstest]
    #[tokio::test]
    async fn test_get_storage_at_fails_contract_not_exists() {
        let client = get_test_context().await.rpc_client();

        let storage_response = client.get_storage_at(felt!("0x123"), DEVNET_ACCOUNTS[0].0, BlockId::Number(1)).await;
        assert!(matches!(storage_response, Err(ProviderError::StarknetError(StarknetError::ContractNotFound))));
    }

    #[rstest]
    #[tokio::test]
    async fn test_get_storage_at_fails_block_not_exists() {
        let client = get_test_context().await.rpc_client();

        let storage_response = client.get_storage_at(STRK_CONTRACT_ADDRESS, felt!("0x0"), BlockId::Number(4)).await;
        assert!(matches!(storage_response, Err(ProviderError::StarknetError(StarknetError::BlockNotFound))));
    }

    /// Retrieves the state update for a specific block.
    #[rstest]
    #[tokio::test]
    async fn test_get_state_update_works() {
        let client = get_test_context().await.rpc_client();

        let state_update =
            client.get_state_update(BlockId::Number(2)).await.expect("Failed to get state update for block number 2");
        let state_update = match state_update {
            MaybePendingStateUpdate::Update(mut state_update) => {
                state_update.state_diff.storage_diffs.sort_by(|a, b| a.address.cmp(&b.address));
                state_update.state_diff.storage_diffs.iter_mut().for_each(|storage_diff| {
                    storage_diff.storage_entries.sort_by(|a, b| a.key.cmp(&b.key));
                });
                state_update.state_diff.nonces.sort_by(|a, b| a.contract_address.cmp(&b.contract_address));
                MaybePendingStateUpdate::Update(state_update)
            }
            _ => unreachable!("State update at block 2 should not be pending"),
        };

        let expected_state_update = MaybePendingStateUpdate::Update(StateUpdate {
            block_hash: felt!("0x351eb09995bec11a120aa85cae65d02052e9a19fe4e17c864c65d224aa0fdf1"),
            old_root: felt!("0x1c9caa6ebf140623100d04ad0c923fc59824921e022a501756cefe9d0c4ba85"),
            new_root: felt!("0x4f6413432d5dfc251371f5ed84e23e5a1bbaa08e3b2cc4f1694568af26dc954"),
            state_diff: StateDiff {
                storage_diffs: vec![ContractStorageDiffItem {
                    address: felt!("0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7"),
                    storage_entries: vec![
                        StorageEntry {
                            key: felt!("0x4094ba053dbf01d3f61e89added60b79ec71874d99c20e47cf2a4244de9894"),
                            value: felt!("0x21e19e0c9bab23ffc34"),
                        },
                        StorageEntry {
                            key: felt!("0x5c9f465f699f4b0b12df48940c006bf64af23c4fa77aeaa17a7389bab8fb0f8"),
                            value: felt!("0x21e19e0c9bab23ffc34"),
                        },
                        StorageEntry {
                            key: felt!("0x5f0ff778b7573f876383a98d7558058ce30c4155e55ddf4d69fb2a431a4fc8d"),
                            value: felt!("0x21e19e0c9bab23ffdf8"),
                        },
                        StorageEntry {
                            key: felt!("0x709c6298f62a9011c05499b9f5ccce4ecc3e0753e48096edef484c409c25181"),
                            value: felt!("0x1940"),
                        },
                        StorageEntry {
                            key: felt!("0x7eb7e3bef0b949044c0df9180e580119422c62be527491fdc06212c94bf4109"),
                            value: felt!("0x21e19e0c9bab23ffc34"),
                        },
                    ],
                }],
                deprecated_declared_classes: vec![],
                declared_classes: vec![],
                deployed_contracts: vec![],
                replaced_classes: vec![],
                nonces: vec![
                    NonceUpdate { contract_address: DEVNET_ACCOUNTS[1].0, nonce: felt!("0x2") },
                    NonceUpdate { contract_address: DEVNET_ACCOUNTS[0].0, nonce: felt!("0x2") },
                    NonceUpdate { contract_address: DEVNET_ACCOUNTS[2].0, nonce: felt!("0x2") },
                ],
            },
        });

        assert_eq!(state_update, expected_state_update);
    }

    #[rstest]
    #[tokio::test]
    async fn test_get_state_update_fails_block_not_exists() {
        let client = get_test_context().await.rpc_client();

        let state_update = client.get_state_update(BlockId::Number(4)).await;
        assert!(matches!(state_update, Err(ProviderError::StarknetError(StarknetError::BlockNotFound))));
    }

    /// Retrieves events based on a filter.
    ///
    /// Note: The response from juno contains different continuation token from madara. Only spec we
    /// need to follow is that the continuation token should be of type string.
    ///
    /// In here we are testing the event data apart from continuation token and checking type for
    /// the continuation token.
    #[rstest]
    #[tokio::test]
    async fn test_get_events_works() {
        let client = get_test_context().await.rpc_client();

        let events = {
            client
                .get_events(
                    EventFilter {
                        from_block: Some(BlockId::Number(1)),
                        to_block: Some(BlockId::Number(2)),
                        address: Some(ETH_CONTRACT_ADDRESS),
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
                    from_address: ETH_CONTRACT_ADDRESS,
                    keys: vec![
                        felt!("0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9"),
                        DEVNET_ACCOUNTS[0].0,
                        felt!("0x123"),
                    ],
                    data: vec![felt!("0x226"), felt!("0x0")],
                    block_hash: Some(felt!("0x7fec16502476dad04e30fd8042942441af53b58b7142ac639017fd959c9879f")),
                    block_number: Some(1),
                    transaction_hash: felt!("0x7fa75444e0a4138c927b0c1a65b6a14959ddc3f592369a9381674a2bbbbbae1"),
                },
                EmittedEvent {
                    from_address: ETH_CONTRACT_ADDRESS,
                    keys: vec![
                        felt!("0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9"),
                        DEVNET_ACCOUNTS[1].0,
                        felt!("0x123"),
                    ],

                    data: vec![felt!("0x226"), felt!("0x0")],
                    block_hash: Some(felt!("0x7fec16502476dad04e30fd8042942441af53b58b7142ac639017fd959c9879f")),
                    block_number: Some(1),
                    transaction_hash: felt!("0x5c49e4eb18cf49d76e3b653b82d8662b93640bb70b019b165e5a733c19e2657"),
                },
            ],
            continuation_token: Some("1-2".to_string()),
        };

        assert_eq!(events.events, expected_events.events);
        assert_type_equality(&events.continuation_token, &expected_events.continuation_token);
    }

    /// Retrieves events based on a filter with a continuation token.
    #[rstest]
    #[tokio::test]
    async fn test_get_events_with_continuation_token_works() {
        let client = get_test_context().await.rpc_client();

        let events = {
            client
                .get_events(
                    EventFilter {
                        from_block: Some(BlockId::Number(2)),
                        to_block: Some(BlockId::Number(2)),
                        address: Some(ETH_CONTRACT_ADDRESS),
                        keys: Some(vec![vec![]]),
                    },
                    Some("1-2".to_string()),
                    2,
                )
                .await
                .unwrap()
        };

        let expected_events = EventsPage {
            events: vec![
                EmittedEvent {
                    from_address: ETH_CONTRACT_ADDRESS,
                    keys: vec![
                        felt!("0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9"),
                        DEVNET_ACCOUNTS[2].0,
                        felt!("0x123"),
                    ],
                    data: vec![felt!("0x226"), felt!("0x0")],
                    block_hash: Some(felt!("0x7fec16502476dad04e30fd8042942441af53b58b7142ac639017fd959c9879f")),
                    block_number: Some(1),
                    transaction_hash: felt!("0x6042a5eac0ed230bb85fa7854ab96f3172ef86a43e50eae9cbe0ecfc1dddc23"),
                },
                EmittedEvent {
                    from_address: ETH_CONTRACT_ADDRESS,
                    keys: vec![
                        felt!("0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9"),
                        DEVNET_ACCOUNTS[3].0,
                        felt!("0x123"),
                    ],
                    data: vec![felt!("0x226"), felt!("0x0")],
                    block_hash: Some(felt!("0x7fec16502476dad04e30fd8042942441af53b58b7142ac639017fd959c9879f")),
                    block_number: Some(1),
                    transaction_hash: felt!("0x47adaedd07e0cb1f37d6d578a041ec9da336f0eb29df8b123e2c17759afe9a6"),
                },
            ],
            continuation_token: Some("1-4".to_string()),
        };

        assert_eq!(events.events, expected_events.events);
        assert_type_equality(&events.continuation_token, &expected_events.continuation_token);
    }

    #[rstest]
    #[tokio::test]
    async fn test_get_events_fails_block_not_exists() {
        let client = get_test_context().await.rpc_client();

        let result = client
            .get_events(
                EventFilter {
                    from_block: Some(BlockId::Number(4)),
                    to_block: Some(BlockId::Number(4)),
                    address: Some(ETH_CONTRACT_ADDRESS),
                    keys: Some(vec![vec![]]),
                },
                None,
                2,
            )
            .await;
        assert!(matches!(result, Err(ProviderError::StarknetError(StarknetError::BlockNotFound))));
    }

    #[rstest]
    #[tokio::test]
    async fn test_get_events_fails_to_block_bigger_than_from_block() {
        let client = get_test_context().await.rpc_client();

        let result = client
            .get_events(
                EventFilter {
                    from_block: Some(BlockId::Number(2)),
                    to_block: Some(BlockId::Number(1)),
                    address: Some(ETH_CONTRACT_ADDRESS),
                    keys: Some(vec![vec![]]),
                },
                None,
                2,
            )
            .await;
        assert!(matches!(result, Err(ProviderError::StarknetError(StarknetError::BlockNotFound))));
    }

    #[rstest]
    #[tokio::test]
    async fn test_get_events_fails_invalid_continuation_token() {
        let client = get_test_context().await.rpc_client();

        let result = client
            .get_events(
                EventFilter {
                    from_block: Some(BlockId::Number(1)),
                    to_block: Some(BlockId::Number(2)),
                    address: Some(ETH_CONTRACT_ADDRESS),
                    keys: None,
                },
                Some("invalid".to_string()),
                2,
            )
            .await;
        assert!(matches!(result, Err(ProviderError::StarknetError(StarknetError::InvalidContinuationToken))));
    }

    /// Calls a contract function at a specific block.
    #[rstest]
    #[tokio::test]
    async fn test_call_works() {
        let client = get_test_context().await.rpc_client();

        let call_response = {
            client
                .call(
                    FunctionCall {
                        contract_address: STRK_CONTRACT_ADDRESS,
                        entry_point_selector: selector!("balance_of"),
                        calldata: vec![DEVNET_ACCOUNTS[0].0],
                    },
                    BlockId::Number(1),
                )
                .await
                .unwrap()
        };

        let expected_call_response = vec![felt!("0x21e19e0c9bab23ffff6"), felt!("0x0")];

        assert_eq!(call_response, expected_call_response);
    }

    #[rstest]
    #[tokio::test]
    async fn test_call_fails_block_not_exists() {
        let client = get_test_context().await.rpc_client();

        let call_response = client
            .call(
                FunctionCall {
                    contract_address: STRK_CONTRACT_ADDRESS,
                    entry_point_selector: selector!("balance_of"),
                    calldata: vec![DEVNET_ACCOUNTS[0].0],
                },
                BlockId::Number(4),
            )
            .await;
        assert!(matches!(call_response, Err(ProviderError::StarknetError(StarknetError::BlockNotFound))));
    }

    #[rstest]
    #[tokio::test]
    async fn test_call_fails_contract_not_exists() {
        let client = get_test_context().await.rpc_client();

        let call_response = client
            .call(
                FunctionCall {
                    contract_address: felt!("0x123"),
                    entry_point_selector: selector!("balance_of"),
                    calldata: vec![DEVNET_ACCOUNTS[0].0],
                },
                BlockId::Number(1),
            )
            .await;
        assert!(matches!(call_response, Err(ProviderError::StarknetError(StarknetError::ContractNotFound))));
    }

    #[rstest]
    #[tokio::test]
    async fn test_call_fails_entry_point_not_exists() {
        let client = get_test_context().await.rpc_client();

        let call_response = client
            .call(
                FunctionCall {
                    contract_address: STRK_CONTRACT_ADDRESS,
                    entry_point_selector: selector!("invalid_entry_point"),
                    calldata: vec![DEVNET_ACCOUNTS[0].0],
                },
                BlockId::Number(1),
            )
            .await;
        assert!(matches!(
            call_response,
            Err(ProviderError::StarknetError(StarknetError::TransactionExecutionError(..)))
        ));
    }

    /// Note: In this test case we are comparing the decompressed program output that we get from this js script
    /// with the decompressed program we get from the madara response.
    ///
    /// Along with that we are also checking for abi and the entry points.
    ///
    /// Note: In this test case we are comparing the decompressed program output that we get from this js script
    /// with the decompressed program we get from the madara response.
    ///
    /// Along with that we are also checking for abi and the entry points.
    #[rstest]
    #[tokio::test]
    async fn test_get_class_works() {
        let client = get_test_context().await.rpc_client();

        let contract_class = {
            client
                .get_class(
                    BlockId::Number(1),
                    felt!("0x40b9e69e14ddc34a98ec8133c80807c144b818bc6cbf5a119d8f62535258142"),
                )
                .await
                .unwrap()
        };

        compare_contract_class(contract_class);
    }

    /// Retrieves the compiled contract class for a given class hash.
    /// The class must be a Sierra contract class.
    ///
    /// Note: The `compiler_version` field is ignored in the comparison because,
    /// for Sierra class versions > v1.1.0, we always use the latest version of the compiler.
    #[rstest]
    #[tokio::test]
    async fn test_get_compiled_casm_works() {
        let url = get_test_context().await.madara.rpc_url.clone();

        // use a reqwest client because starknet-providers does not support the rpc call yet
        let client = reqwest::Client::new();

        // use the v0.8.0 rpc endpoint
        let url = url.join("rpc/v0_8_0/").unwrap();

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
    /// Note: The program has been compressed using the same script mentioned in the above test case.
    // TODO: fix the test with STARKNET_TOKEN_CLASS
    #[ignore]
    #[rstest]
    #[tokio::test]
    async fn test_get_class_at_works() {
        let client = get_test_context().await.rpc_client();

        let contract_class = {
            client
                .get_class_at(
                    BlockId::Number(1),
                    felt!("0x043abaa073c768ebf039c0c4f46db9acc39e9ec165690418060a652aab39e7d8"),
                )
                .await
                .unwrap()
        };

        compare_contract_class(contract_class);
    }

    /// Estimates the fee for a transaction or a list of transactions.
    ///
    /// Note: The test is ignored as of now because madara support the transaction with starknet version >= 0.13.0
    /// and ideally for the node will call the sequencer to get the result of the transaction for older starknet version.
    #[rstest]
    #[tokio::test]
    async fn test_estimate_fee_works() {
        let client = get_test_context().await.rpc_client();

        let call_response = {
            client
                .estimate_fee(
                    vec![BroadcastedTransaction::Invoke(BroadcastedInvokeTransaction::V1(
                        BroadcastedInvokeTransactionV1 {
                            sender_address: DEVNET_ACCOUNTS[0].0,
                            calldata: vec![
                                felt!("0x1"),
                                STRK_CONTRACT_ADDRESS,
                                selector!("transfer"),
                                felt!("0x03"),
                                DEVNET_ACCOUNTS[1].0,
                                felt!("0x0a"),
                                felt!("0x0"),
                            ],
                            max_fee: felt!("0x186a0"),
                            signature: vec![
                                felt!("0x25e3ef7815d3f8fc82d572d3a48b8d7fff574d9cb6b58cab0f9871f91c8d6d5"),
                                felt!("0x5ed550cac383cd158845cc375888569cce5c2e81f54eb26b08dc0aa3e035986"),
                            ],
                            nonce: felt!("0x1"),
                            is_query: false,
                        },
                    ))],
                    vec![SimulationFlagForEstimateFee::SkipValidate],
                    BlockId::Number(1),
                )
                .await
                .unwrap()
        };

        let expected_call_response: Vec<FeeEstimate> = vec![FeeEstimate {
            gas_consumed: felt!("0x16"),
            gas_price: felt!("0x0a"),
            data_gas_consumed: felt!("0x0140"),
            data_gas_price: felt!("0x1"),
            overall_fee: felt!("0x021c"),
            unit: PriceUnit::Wei,
        }];
        assert_eq!(call_response, expected_call_response);
    }

    #[rstest]
    #[tokio::test]
    async fn test_estimate_fee_fails_account_not_exists() {
        let client = get_test_context().await.rpc_client();

        let call_response = client
            .estimate_fee(
                vec![BroadcastedTransaction::Invoke(BroadcastedInvokeTransaction::V1(
                    BroadcastedInvokeTransactionV1 {
                        sender_address: felt!("0x123"),
                        calldata: vec![
                            felt!("0x1"),
                            STRK_CONTRACT_ADDRESS,
                            selector!("transfer"),
                            felt!("0x03"),
                            DEVNET_ACCOUNTS[1].0,
                            felt!("0x0a"),
                            felt!("0x0"),
                        ],
                        max_fee: felt!("0x186a0"),
                        signature: vec![
                            felt!("0x25e3ef7815d3f8fc82d572d3a48b8d7fff574d9cb6b58cab0f9871f91c8d6d5"),
                            felt!("0x5ed550cac383cd158845cc375888569cce5c2e81f54eb26b08dc0aa3e035986"),
                        ],
                        nonce: felt!("0x1"),
                        is_query: false,
                    },
                ))],
                vec![SimulationFlagForEstimateFee::SkipValidate],
                BlockId::Number(1),
            )
            .await;
        assert!(matches!(call_response, Err(ProviderError::StarknetError(StarknetError::ContractError(..)))));
    }

    /// Estimates the fee for a message from L1 to L2.
    ///
    /// Note: The test is ignored as of now because madara support the transaction with starknet version >= 0.13.0
    /// and ideally for the node will call the sequencer to get the result of the transaction for older starknet version.
    #[ignore]
    #[rstest]
    #[tokio::test]
    async fn test_estimate_message_fee_works() {
        let client = get_test_context().await.rpc_client();

        let call_response = {
            client
                .estimate_message_fee(
                    MsgFromL1 {
                        from_address: EthAddress::from_hex("0x8453fc6cd1bcfe8d4dfc069c400b433054d47bdc").unwrap(),
                        to_address: felt!("0x04c5772d1914fe6ce891b64eb35bf3522aeae1315647314aac58b01137607f3f"),
                        entry_point_selector: felt!(
                            "0x2d757788a8d8d6f21d1cd40bce38a8222d70654214e96ff95d8086e684fbee5"
                        ),
                        payload: vec![felt!("0x8453fc6cd1bcfe8d4dfc069c400b433054d47bdc"), felt!("0x0"), felt!("0x0")],
                    },
                    BlockId::Number(1),
                )
                .await
                .unwrap()
        };

        let expected_call_response: FeeEstimate = FeeEstimate {
            gas_consumed: felt!("0x4424"),
            gas_price: felt!("0x33dda9da0"),
            data_gas_consumed: felt!("0x0"),
            data_gas_price: felt!("0x1"),
            overall_fee: felt!("0xdce2c49caa80"),
            unit: PriceUnit::Wei,
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
