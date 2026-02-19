use crate::{wait_for_cond, MadaraCmdBuilder};
pub use crate::{ACCOUNTS, ACCOUNT_SECRETS};
use anyhow::ensure;
use rstest::rstest;
use starknet::accounts::{Account, ExecutionEncoding, SingleOwnerAccount};
use starknet::signers::{LocalWallet, SigningKey};
use starknet_core::types::{BlockId, BlockTag, Call, Felt};
use starknet_core::utils::starknet_keccak;
use starknet_providers::Provider;
use std::time::Duration;

pub const SEQUENCER_ADDRESS: Felt = Felt::from_hex_unchecked("0x123");

pub const ERC20_STRK_CONTRACT_ADDRESS: Felt =
    Felt::from_hex_unchecked("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d");
#[allow(unused)]
pub const ERC20_ETH_CONTRACT_ADDRESS: Felt =
    Felt::from_hex_unchecked("0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7");

pub const ACCOUNT_SECRET: Felt = ACCOUNT_SECRETS[0];
pub const ACCOUNT_ADDRESS: Felt = ACCOUNTS[0];

#[rstest]
#[tokio::test]
async fn madara_devnet_add_transaction() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let args = &[
        "--devnet",
        "--no-l1-sync",
        "--l1-gas-price",
        "1",
        "--blob-gas-price",
        "1",
        "--chain-config-override",
        "block_time=1s",
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
            ensure!(receipt.block.is_block());
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
            ensure!(receipt.block.is_block());
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
        "--l1-gas-price",
        "0",
        "--blob-gas-price",
        "0",
        // never produce blocks & pending txs
        "--chain-config-path",
        "test_devnet.yaml",
        "--chain-config-override",
        "block_time=5min",
    ]);
    let mut node = cmd_builder.clone().run();
    node.wait_for_ready().await;

    let chain_id = node.json_rpc().chain_id().await.unwrap();

    let signer = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(ACCOUNT_SECRET));
    let mut account =
        SingleOwnerAccount::new(node.json_rpc(), signer, ACCOUNT_ADDRESS, chain_id, ExecutionEncoding::New);
    account.set_block_id(BlockId::Tag(BlockTag::PreConfirmed));

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
        "--l1-gas-price",
        "0",
        "--blob-gas-price",
        "0",
        // never produce blocks but produce pending txs
        "--chain-config-path",
        "test_devnet.yaml",
        "--chain-config-override",
        "block_time=5min",
    ]);
    let mut node = cmd_builder.clone().run();
    node.wait_for_ready().await;

    // tx should be in mempool

    wait_for_cond(
        || async {
            let _receipt = node.json_rpc().get_transaction_receipt(res.transaction_hash).await?;
            Ok(())
        },
        Duration::from_millis(500),
        60,
    )
    .await;
}
