use rstest::rstest;
use starknet::accounts::{Account, ExecutionEncoding, SingleOwnerAccount};
use starknet::signers::{LocalWallet, SigningKey};
use starknet_core::types::{BlockId, BlockTag, Call, Felt, ReceiptBlock};
use starknet_core::utils::starknet_keccak;
use starknet_providers::Provider;
use std::time::Duration;

use crate::{wait_for_cond, MadaraCmdBuilder};

const ERC20_STRK_CONTRACT_ADDRESS: Felt =
    Felt::from_hex_unchecked("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d");
#[allow(unused)]
const ERC20_ETH_CONTRACT_ADDRESS: Felt =
    Felt::from_hex_unchecked("0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7");

const ACCOUNT_SECRET: Felt =
    Felt::from_hex_unchecked("0x077e56c6dc32d40a67f6f7e6625c8dc5e570abe49c0a24e9202e4ae906abcc07");
const ACCOUNT_ADDRESS: Felt =
    Felt::from_hex_unchecked("0x055be462e718c4166d656d11f89e341115b8bc82389c3762a10eade04fcb225d");

#[rstest]
#[tokio::test]
async fn madara_devnet_add_transaction() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let args = &[
        "--devnet",
        "--no-l1-sync",
        "--gas-price",
        "0",
        // only produce blocks no pending txs
        "--chain-config-override",
        "block_time=1s,pending_block_update_time=1s",
    ];

    let cmd_builder = MadaraCmdBuilder::new().args(*args);
    let mut node = cmd_builder.run();
    node.wait_for_ready().await;

    let chain_id = node.json_rpc().chain_id().await.unwrap();

    let signer = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(ACCOUNT_SECRET));
    let mut account =
        SingleOwnerAccount::new(node.json_rpc(), signer, ACCOUNT_ADDRESS, chain_id, ExecutionEncoding::New);
    account.set_block_id(BlockId::Tag(BlockTag::Latest));

    let res = account
        .execute_v3(vec![Call {
            to: ERC20_STRK_CONTRACT_ADDRESS,
            selector: starknet_keccak(b"transfer"),
            calldata: vec![ACCOUNT_ADDRESS, 15.into(), Felt::ZERO],
        }])
        .send()
        .await
        .unwrap();

    wait_for_cond(
        || async {
            let receipt = node.json_rpc().get_transaction_receipt(res.transaction_hash).await?;
            assert!(receipt.block.is_block());
            Ok(())
        },
        Duration::from_millis(500),
        60,
    )
    .await;

    tokio::time::sleep(Duration::from_secs(2)).await;

    let res = account
        .execute_v3(vec![Call {
            to: ERC20_STRK_CONTRACT_ADDRESS,
            selector: starknet_keccak(b"transfer"),
            calldata: vec![ACCOUNT_ADDRESS, 40.into(), Felt::ZERO],
        }])
        .send()
        .await
        .unwrap();

    wait_for_cond(
        || async {
            let receipt = node.json_rpc().get_transaction_receipt(res.transaction_hash).await?;
            assert!(receipt.block.is_block());
            Ok(())
        },
        Duration::from_millis(500),
        60,
    )
    .await;
}

#[rstest]
#[tokio::test]
async fn madara_devnet_mempool_saving() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let cmd_builder = MadaraCmdBuilder::new().args([
        "--devnet",
        "--no-l1-sync",
        "--gas-price",
        "0",
        // never produce blocks & pending txs
        "--chain-config-path",
        "test_devnet.yaml",
        "--chain-config-override",
        "block_time=5min,pending_block_update_time=5min",
    ]);
    let mut node = cmd_builder.clone().run();
    node.wait_for_ready().await;

    let chain_id = node.json_rpc().chain_id().await.unwrap();

    let signer = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(ACCOUNT_SECRET));
    let mut account =
        SingleOwnerAccount::new(node.json_rpc(), signer, ACCOUNT_ADDRESS, chain_id, ExecutionEncoding::New);
    account.set_block_id(BlockId::Tag(BlockTag::Pending));

    let res = account
        .execute_v3(vec![Call {
            to: ERC20_STRK_CONTRACT_ADDRESS,
            selector: starknet_keccak(b"transfer"),
            calldata: vec![ACCOUNT_ADDRESS, 15.into(), Felt::ZERO],
        }])
        .send()
        .await
        .unwrap();

    drop(node);

    // tx should be in saved mempool

    let cmd_builder = cmd_builder.args([
        "--devnet",
        "--no-l1-sync",
        "--gas-price",
        "0",
        // never produce blocks but produce pending txs
        "--chain-config-path",
        "test_devnet.yaml",
        "--chain-config-override",
        "block_time=5min,pending_block_update_time=500ms",
    ]);
    let mut node = cmd_builder.clone().run();
    node.wait_for_ready().await;

    // tx should be in mempool

    wait_for_cond(
        || async {
            let receipt = node.json_rpc().get_transaction_receipt(res.transaction_hash).await?;
            assert_eq!(receipt.block, ReceiptBlock::Pending);
            Ok(())
        },
        Duration::from_millis(500),
        60,
    )
    .await;
}
