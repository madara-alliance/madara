use crate::{wait_for_cond, MadaraCmdBuilder};
use rstest::rstest;
use starknet::accounts::{Account, ExecutionEncoding, SingleOwnerAccount};
use starknet::signers::{LocalWallet, SigningKey};
use starknet_core::types::{BlockId, BlockTag, Call, Felt, ReceiptBlock};
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

/// Madara default devnet accounts.
pub const ACCOUNTS: [Felt; 10] = [
    Felt::from_hex_unchecked("0x055be462e718c4166d656d11f89e341115b8bc82389c3762a10eade04fcb225d"),
    Felt::from_hex_unchecked("0x008a1719e7ca19f3d91e8ef50a48fc456575f645497a1d55f30e3781f786afe4"),
    Felt::from_hex_unchecked("0x0733a8e2bcced14dcc2608462bd96524fb64eef061689b6d976708efc2c8ddfd"),
    Felt::from_hex_unchecked("0x025073e0772b1e348a5da66ea67fb46f75ecdca1bd24dbbc98567cbf4a0e00b3"),
    Felt::from_hex_unchecked("0x0294f066a54e07616fd0d50c935c2b5aa616d33631fec94b34af8bd4f6296f68"),
    Felt::from_hex_unchecked("0x005d1d65ea82aa0107286e68537adf0371601789e26b1cd6e455a8e5be5c5665"),
    Felt::from_hex_unchecked("0x01d775883a0a6e5405a345f18d7639dcb54b212c362d5a99087f742fba668396"),
    Felt::from_hex_unchecked("0x04add50f5bcc31a8418b43b1ddc8d703986094baf998f8e9625e13dbcc3df18b"),
    Felt::from_hex_unchecked("0x03dbe3dd8c2f721bc24e87bcb739063a10ee738cef090bc752bc0d5a29f10b72"),
    Felt::from_hex_unchecked("0x07484e8e3af210b2ead47fa08c96f8d18b616169b350a8b75fe0dc4d2e01d493"),
];

/// Private keys for the devnet accounts.
pub const ACCOUNT_SECRETS: [Felt; 10] = [
    Felt::from_hex_unchecked("0x077e56c6dc32d40a67f6f7e6625c8dc5e570abe49c0a24e9202e4ae906abcc07"),
    Felt::from_hex_unchecked("0x0514977443078cf1e0c36bc88b89ada9a46061a5cf728f40274caea21d76f174"),
    Felt::from_hex_unchecked("0x00177100ae65c71074126963e695e17adf5b360146f960378b5cdfd9ed69870b"),
    Felt::from_hex_unchecked("0x07ae55c8093920562c1cbab9edeb4eb52f788b93cac1d5721bda20c96100d743"),
    Felt::from_hex_unchecked("0x02ce1754eb64b7899c64dcdd0cff138864be2514e70e7761c417b728f2bf7457"),
    Felt::from_hex_unchecked("0x037a683c3969bf18044c9d2bbe0b1739897c89cf25420342d6dfc36c30fc519d"),
    Felt::from_hex_unchecked("0x07b4a2263d9cc475816a03163df7efd58552f1720c8df0bd2a813663895ef022"),
    Felt::from_hex_unchecked("0x064b37f84e667462b95dc56e3c5e93a703ef16d73de7b9c5bfd92b90f11f90e1"),
    Felt::from_hex_unchecked("0x0213d0d77d5ff9ffbeabdde0af7513e89aafd5e36ae99b8401283f6f57c57696"),
    Felt::from_hex_unchecked("0x0410c6eadd73918ea90b6658d24f5f2c828e39773819c1443d8602a3c72344c2"),
];

#[rstest]
#[tokio::test]
async fn madara_devnet_add_transaction() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let args = &[
        "--devnet",
        "--no-l1-sync",
        "--l1-gas-price",
        "0",
        "--blob-gas-price",
        "0",
        // only produce blocks no pending txs
        "--chain-config-override",
        "block_time=1s,pending_block_update_time=null",
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
        "--l1-gas-price",
        "0",
        "--blob-gas-price",
        "0",
        // never produce blocks & pending txs
        "--chain-config-path",
        "test_devnet.yaml",
        "--chain-config-override",
        "block_time=5min,pending_block_update_time=null",
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
        "--l1-gas-price",
        "0",
        "--blob-gas-price",
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
