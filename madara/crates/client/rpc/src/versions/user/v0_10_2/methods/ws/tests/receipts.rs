use super::*;

#[tokio::test]
async fn subscribe_new_transaction_receipts_confirmed_filter_and_sender_v0_10_2() {
    let (backend, starknet) = rpc_test_setup();
    let (_handle, server_url) = start_server(starknet).await;
    let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

    let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_new_transaction_receipts(
        &client,
        Some(vec![FinalityStatus::AcceptedOnL2]),
        Some(vec![SENDER_ADDRESS]),
    )
    .await
    .expect("Failed subscription");

    let transaction_hash = Felt::from_hex_unchecked("0x4242");
    let block_info = backend.write_access().add_full_block_with_classes(
        &FullBlockWithoutCommitments {
            header: PreconfirmedHeader { block_number: 0, ..Default::default() },
            state_diff: Default::default(),
            transactions: vec![
                transaction_with_receipt(OTHER_SENDER_ADDRESS, Felt::from_hex_unchecked("0x4141")),
                transaction_with_receipt(SENDER_ADDRESS, transaction_hash),
            ],
            events: vec![],
        },
        &[],
        true,
    );
    let block_hash = block_info.expect("Failed to store confirmed block");

    let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("Timed out waiting for receipt")
        .expect("Subscription closed unexpectedly")
        .expect("Failed to retrieve receipt");

    assert_eq!(
        item,
        mp_rpc::v0_10_2::TxnReceiptWithBlockInfo {
            transaction_receipt: transaction_with_receipt(SENDER_ADDRESS, transaction_hash)
                .receipt
                .to_rpc_v0_10(mp_rpc::v0_10_2::TxnFinalityStatus::L2),
            block_hash: Some(block_hash.block_hash),
            block_number: 0,
        }
    );
}

#[tokio::test]
async fn subscribe_new_transaction_receipts_l1_confirmed_block_v0_10_2() {
    let (backend, starknet) = rpc_test_setup();
    let (_handle, server_url) = start_server(starknet).await;
    let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

    let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_new_transaction_receipts(
        &client,
        Some(vec![FinalityStatus::AcceptedOnL2]),
        Some(vec![SENDER_ADDRESS]),
    )
    .await
    .expect("Failed subscription");

    backend.set_latest_l1_confirmed(Some(0)).expect("Failed to set L1 confirmed block");
    let transaction_hash = Felt::from_hex_unchecked("0x4444");
    let tx = transaction_with_receipt(SENDER_ADDRESS, transaction_hash);
    let block_hash = backend
        .write_access()
        .add_full_block_with_classes(
            &FullBlockWithoutCommitments {
                header: PreconfirmedHeader { block_number: 0, ..Default::default() },
                state_diff: Default::default(),
                transactions: vec![tx.clone()],
                events: vec![],
            },
            &[],
            true,
        )
        .expect("Failed to store confirmed block")
        .block_hash;

    let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("Timed out waiting for receipt")
        .expect("Subscription closed unexpectedly")
        .expect("Failed to retrieve receipt");

    assert_eq!(
        item,
        mp_rpc::v0_10_2::TxnReceiptWithBlockInfo {
            transaction_receipt: tx.receipt.to_rpc_v0_10(mp_rpc::v0_10_2::TxnFinalityStatus::L1),
            block_hash: Some(block_hash),
            block_number: 0,
        }
    );
}

#[tokio::test]
async fn subscribe_new_transaction_receipts_l1_confirmed_block_v0_10_0() {
    let (backend, starknet) = rpc_test_setup();
    let (_handle, server_url) = start_server_v0_10_0(starknet).await;
    let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

    let mut sub = StarknetWsRpcApiV0_10_0Client::subscribe_new_transaction_receipts(
        &client,
        Some(vec![mp_rpc::v0_10_0::FinalityStatus::AcceptedOnL2]),
        Some(vec![SENDER_ADDRESS]),
    )
    .await
    .expect("Failed subscription");

    backend.set_latest_l1_confirmed(Some(0)).expect("Failed to set L1 confirmed block");
    let transaction_hash = Felt::from_hex_unchecked("0x4545");
    let tx = transaction_with_receipt(SENDER_ADDRESS, transaction_hash);
    let block_hash = backend
        .write_access()
        .add_full_block_with_classes(
            &FullBlockWithoutCommitments {
                header: PreconfirmedHeader { block_number: 0, ..Default::default() },
                state_diff: Default::default(),
                transactions: vec![tx.clone()],
                events: vec![],
            },
            &[],
            true,
        )
        .expect("Failed to store confirmed block")
        .block_hash;

    let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("Timed out waiting for receipt")
        .expect("Subscription closed unexpectedly")
        .expect("Failed to retrieve receipt");

    let item = serde_json::to_value(item).expect("Failed to serialize receipt item");
    let expected = serde_json::to_value(mp_rpc::v0_10_0::TxnReceiptWithBlockInfo {
        transaction_receipt: tx.receipt.to_rpc_v0_10(mp_rpc::v0_10_0::TxnFinalityStatus::L1),
        block_hash: Some(block_hash),
        block_number: 0,
    })
    .expect("Failed to serialize expected receipt");

    assert_eq!(item, expected);
}

#[tokio::test]
async fn subscribe_new_transaction_receipts_preconfirmed_filter_and_sender_v0_10_2() {
    let (backend, starknet) = rpc_test_setup();
    let (_handle, server_url) = start_server(starknet).await;
    let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

    let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_new_transaction_receipts(
        &client,
        Some(vec![FinalityStatus::PreConfirmed]),
        Some(vec![SENDER_ADDRESS]),
    )
    .await
    .expect("Failed subscription");

    let transaction_hash = Felt::from_hex_unchecked("0x4343");
    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new_with_content(
            PreconfirmedHeader { block_number: 0, protocol_version: StarknetVersion::V0_13_2, ..Default::default() },
            vec![PreconfirmedExecutedTransaction {
                transaction: transaction_with_receipt(SENDER_ADDRESS, transaction_hash),
                state_diff: Default::default(),
                declared_class: None,
                arrived_at: Default::default(),
                paid_fee_on_l1: None,
            }],
            vec![],
        ))
        .expect("Failed to store preconfirmed block");

    let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("Timed out waiting for receipt")
        .expect("Subscription closed unexpectedly")
        .expect("Failed to retrieve receipt");

    assert_eq!(
        item,
        mp_rpc::v0_10_2::TxnReceiptWithBlockInfo {
            transaction_receipt: transaction_with_receipt(SENDER_ADDRESS, transaction_hash)
                .receipt
                .to_rpc_v0_10(mp_rpc::v0_10_2::TxnFinalityStatus::PreConfirmed),
            block_hash: None,
            block_number: 0,
        }
    );
}

#[tokio::test]
async fn subscribe_new_transaction_receipts_preconfirmed_append_v0_10_2() {
    let (backend, starknet) = rpc_test_setup();
    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader {
            block_number: 0,
            protocol_version: StarknetVersion::V0_13_2,
            ..Default::default()
        }))
        .expect("Failed to create empty preconfirmed block");

    let (_handle, server_url) = start_server(starknet).await;
    let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");
    let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_new_transaction_receipts(
        &client,
        Some(vec![FinalityStatus::PreConfirmed]),
        Some(vec![SENDER_ADDRESS]),
    )
    .await
    .expect("Failed subscription");

    let transaction_hash = Felt::from_hex_unchecked("0x4747");
    let tx = transaction_with_receipt(SENDER_ADDRESS, transaction_hash);
    let executed = vec![PreconfirmedExecutedTransaction {
        transaction: tx.clone(),
        state_diff: Default::default(),
        declared_class: None,
        arrived_at: Default::default(),
        paid_fee_on_l1: None,
    }];
    backend
        .write_access()
        .append_to_preconfirmed(0, &executed, std::iter::empty())
        .expect("Failed to append preconfirmed transaction");

    let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("Timed out waiting for appended preconfirmed receipt")
        .expect("Subscription closed unexpectedly")
        .expect("Failed to retrieve receipt");

    assert_eq!(
        item,
        mp_rpc::v0_10_2::TxnReceiptWithBlockInfo {
            transaction_receipt: tx.receipt.to_rpc_v0_10(mp_rpc::v0_10_2::TxnFinalityStatus::PreConfirmed),
            block_hash: None,
            block_number: 0,
        }
    );
}

#[tokio::test]
async fn subscribe_new_transaction_receipts_preconfirmed_append_v0_10_0() {
    let (backend, starknet) = rpc_test_setup();
    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader {
            block_number: 0,
            protocol_version: StarknetVersion::V0_13_2,
            ..Default::default()
        }))
        .expect("Failed to create empty preconfirmed block");

    let (_handle, server_url) = start_server_v0_10_0(starknet).await;
    let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");
    let mut sub = StarknetWsRpcApiV0_10_0Client::subscribe_new_transaction_receipts(
        &client,
        Some(vec![mp_rpc::v0_10_0::FinalityStatus::PreConfirmed]),
        Some(vec![SENDER_ADDRESS]),
    )
    .await
    .expect("Failed subscription");

    let transaction_hash = Felt::from_hex_unchecked("0x4848");
    let tx = transaction_with_receipt(SENDER_ADDRESS, transaction_hash);
    let executed = vec![PreconfirmedExecutedTransaction {
        transaction: tx.clone(),
        state_diff: Default::default(),
        declared_class: None,
        arrived_at: Default::default(),
        paid_fee_on_l1: None,
    }];
    backend
        .write_access()
        .append_to_preconfirmed(0, &executed, std::iter::empty())
        .expect("Failed to append preconfirmed transaction");

    let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("Timed out waiting for appended preconfirmed receipt")
        .expect("Subscription closed unexpectedly")
        .expect("Failed to retrieve receipt");

    let item = serde_json::to_value(item).expect("Failed to serialize receipt item");
    let expected = serde_json::to_value(mp_rpc::v0_10_0::TxnReceiptWithBlockInfo {
        transaction_receipt: tx.receipt.to_rpc_v0_10(mp_rpc::v0_10_0::TxnFinalityStatus::PreConfirmed),
        block_hash: None,
        block_number: 0,
    })
    .expect("Failed to serialize expected receipt");

    assert_eq!(item, expected);
}

#[tokio::test]
async fn subscribe_new_transaction_receipts_reorg_then_resume_v0_10_2() {
    let (backend, starknet) = rpc_test_setup();
    let (_handle, server_url) = start_server(starknet).await;
    let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

    let (block_0_hash, _block_0) = add_block_at_with_hash(&backend, 0);
    let (block_1_hash, _block_1) = add_block_at_with_hash(&backend, 1);

    let mut sub = raw_subscribe_new_transaction_receipts(&client).await;

    backend.revert_to(&block_0_hash).expect("Revert should succeed");

    let reorg = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("Timed out waiting for reorg notification")
        .expect("Subscription closed unexpectedly")
        .expect("Failed to retrieve reorg notification");

    assert_eq!(
        reorg,
        serde_json::to_value(mp_rpc::v0_10_2::ReorgData {
            starting_block_hash: block_1_hash,
            starting_block_number: 1,
            ending_block_hash: block_1_hash,
            ending_block_number: 1,
        })
        .expect("Failed to serialize expected reorg notification")
    );

    let transaction_hash = Felt::from_hex_unchecked("0xa1a1");
    let tx = transaction_with_receipt(SENDER_ADDRESS, transaction_hash);
    let new_block_hash = backend
        .write_access()
        .add_full_block_with_classes(
            &FullBlockWithoutCommitments {
                header: PreconfirmedHeader {
                    block_number: 1,
                    protocol_version: StarknetVersion::V0_13_2,
                    ..Default::default()
                },
                state_diff: Default::default(),
                transactions: vec![tx.clone()],
                events: vec![],
            },
            &[],
            true,
        )
        .expect("Failed to store replacement confirmed block")
        .block_hash;

    let next = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("Timed out waiting for replacement receipt")
        .expect("Subscription closed unexpectedly")
        .expect("Failed to retrieve replacement receipt");
    let item: mp_rpc::v0_10_2::TxnReceiptWithBlockInfo =
        serde_json::from_value(next).expect("Failed to deserialize replacement receipt item");

    assert_eq!(
        item,
        mp_rpc::v0_10_2::TxnReceiptWithBlockInfo {
            transaction_receipt: tx.receipt.to_rpc_v0_10(mp_rpc::v0_10_2::TxnFinalityStatus::L2),
            block_hash: Some(new_block_hash),
            block_number: 1,
        }
    );
}

#[tokio::test]
async fn subscribe_new_transaction_receipts_preconfirmed_reorg_wins_v0_10_2() {
    let (backend, starknet) = rpc_test_setup();
    let (_handle, server_url) = start_server(starknet).await;
    let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

    let (block_0_hash, _block_0) = add_block_at_with_hash(&backend, 0);
    let (block_1_hash, _block_1) = add_block_at_with_hash(&backend, 1);

    let mut params = ObjectParams::new();
    params.insert("finality_status", vec![FinalityStatus::PreConfirmed]).expect("Building receipt params");
    let mut sub = raw_subscribe_new_transaction_receipts_with_params(&client, params).await;

    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new_with_content(
            PreconfirmedHeader { block_number: 2, protocol_version: StarknetVersion::V0_13_2, ..Default::default() },
            vec![PreconfirmedExecutedTransaction {
                transaction: transaction_with_receipt(SENDER_ADDRESS, Felt::from_hex_unchecked("0x4646")),
                state_diff: Default::default(),
                declared_class: None,
                arrived_at: Default::default(),
                paid_fee_on_l1: None,
            }],
            vec![],
        ))
        .expect("Failed to store preconfirmed block");
    backend.revert_to(&block_0_hash).expect("Revert should succeed");

    let reorg = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("Timed out waiting for reorg notification")
        .expect("Subscription closed unexpectedly")
        .expect("Failed to retrieve reorg notification");

    assert_eq!(
        reorg,
        serde_json::to_value(mp_rpc::v0_10_2::ReorgData {
            starting_block_hash: block_1_hash,
            starting_block_number: 1,
            ending_block_hash: block_1_hash,
            ending_block_number: 1,
        })
        .expect("Failed to serialize expected reorg notification")
    );
}

#[tokio::test]
async fn subscribe_new_transaction_receipts_rejects_too_many_sender_addresses_v0_10_2() {
    let (_backend, starknet) = rpc_test_setup();
    let (_handle, server_url) = start_server(starknet).await;
    let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

    let size = super::super::super::ADDRESS_FILTER_LIMIT as usize + 1;
    let err = StarknetWsRpcApiV0_10_2Client::subscribe_new_transaction_receipts(
        &client,
        Some(vec![FinalityStatus::AcceptedOnL2]),
        Some(vec![SENDER_ADDRESS; size]),
    )
    .await
    .expect_err("Subscription should fail");

    assert_matches!(
        err,
        jsonrpsee::core::client::error::Error::Call(err) => {
            assert_eq!(err, crate::errors::StarknetWsApiError::TooManyAddressesInFilter.into());
        }
    );
}
