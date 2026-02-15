use crate::{
    devnet::{ACCOUNTS, ACCOUNT_ADDRESS, ACCOUNT_SECRET, ERC20_STRK_CONTRACT_ADDRESS},
    wait_for_cond, MadaraCmd, MadaraCmdBuilder,
};
use anyhow::ensure;
use mp_class::FlattenedSierraClass;
use starknet::accounts::{Account, AccountFactory, ExecutionEncoding, OpenZeppelinAccountFactory, SingleOwnerAccount};
use starknet::signers::{LocalWallet, SigningKey};
use starknet_core::types::contract::SierraClass;
use starknet_core::types::{
    BlockId, BlockTag, Call, Felt, MaybePreConfirmedBlockWithTxHashes, MaybePreConfirmedStateUpdate,
    TransactionReceiptWithBlockInfo,
};
use starknet_core::utils::starknet_keccak;
use starknet_providers::Provider;
use std::path::PathBuf;
use std::time::Duration;
use tokio::time::sleep;

const TX_COUNT_TARGET: usize = 100;
const MIN_BLOCK_HEIGHT_TARGET: u64 = 100;
const DEPLOY_ACCOUNT_SECRET: Felt =
    Felt::from_hex_unchecked("0x023f97ee39cb7032589df7f0ba66b92b8a4ed2ea67f9d2e31f2f8de4af61f14");
const DEPLOY_ACCOUNT_SALT: Felt = Felt::from_hex_unchecked("0x123");
const BLOCK_SIGNING_PRIVATE_KEY: &str = "0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdea";

const L2_GAS_PRICE: u128 = 200_000;
const L2_GAS: u64 = 1_000_000_000;
const L1_GAS_PRICE: u128 = 128;
const L1_GAS: u64 = 30_000;
const L1_DATA_GAS_PRICE: u128 = 128;
const L1_DATA_GAS: u64 = 30_000;

macro_rules! with_devnet_fees {
    ($builder:expr) => {
        $builder
            .l2_gas_price(L2_GAS_PRICE)
            .l2_gas(L2_GAS)
            .l1_gas_price(L1_GAS_PRICE)
            .l1_gas(L1_GAS)
            .l1_data_gas_price(L1_DATA_GAS_PRICE)
            .l1_data_gas(L1_DATA_GAS)
    };
}

fn test_devnet_path() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_devnet.yaml").to_string_lossy().into_owned()
}

fn common_args(parallel_merkle: bool) -> Vec<String> {
    let mut args = vec![
        "--devnet".to_string(),
        "--no-l1-sync".to_string(),
        "--l1-gas-price".to_string(),
        "0".to_string(),
        "--blob-gas-price".to_string(),
        "0".to_string(),
        "--devnet-contracts".to_string(),
        "1".to_string(),
        "--chain-config-path".to_string(),
        test_devnet_path(),
        "--chain-config-override".to_string(),
        "block_time=3s".to_string(),
        "--private-key".to_string(),
        BLOCK_SIGNING_PRIVATE_KEY.to_string(),
    ];

    if parallel_merkle {
        args.extend([
            "--parallel-merkle-enabled".to_string(),
            "--parallel-merkle-flush-interval".to_string(),
            "3".to_string(),
            "--parallel-merkle-max-inflight".to_string(),
            "10".to_string(),
            "--parallel-merkle-trie-log-mode".to_string(),
            "checkpoint".to_string(),
        ]);
    }

    args
}

fn extract_block_hash(block: MaybePreConfirmedBlockWithTxHashes) -> Felt {
    match block {
        MaybePreConfirmedBlockWithTxHashes::Block(block) => block.block_hash,
        MaybePreConfirmedBlockWithTxHashes::PreConfirmedBlock(_) => {
            panic!("unexpected pre-confirmed block where confirmed block was expected")
        }
    }
}

fn extract_confirmed_root(update: MaybePreConfirmedStateUpdate) -> Felt {
    match update {
        MaybePreConfirmedStateUpdate::Update(update) => update.new_root,
        MaybePreConfirmedStateUpdate::PreConfirmedUpdate(_) => {
            panic!("unexpected pre-confirmed state update when confirmed update was expected")
        }
    }
}

fn make_transfer_call(to: Felt, amount: u64) -> Call {
    Call {
        to: ERC20_STRK_CONTRACT_ADDRESS,
        selector: starknet_keccak(b"transfer"),
        calldata: vec![to, Felt::from(amount), Felt::ZERO],
    }
}

async fn wait_for_confirmed_receipt<P: Provider + Sync>(
    provider: &P,
    tx_hash: Felt,
) -> TransactionReceiptWithBlockInfo {
    wait_for_cond(
        || async {
            let receipt = provider.get_transaction_receipt(tx_hash).await?;
            ensure!(receipt.block.is_block(), "tx not confirmed yet");
            Ok(receipt)
        },
        Duration::from_millis(250),
        400,
    )
    .await
}

async fn wait_for_block_at_least<P: Provider + Sync>(provider: &P, target_block_n: u64) {
    let _ = wait_for_cond(
        || async {
            let block_n = provider.block_hash_and_number().await?.block_number;
            ensure!(block_n >= target_block_n, "latest block is {block_n}, waiting for >= {target_block_n}");
            Ok(block_n)
        },
        Duration::from_millis(250),
        600,
    )
    .await;
}

async fn start_synced_nodes() -> (MadaraCmd, MadaraCmd) {
    for _ in 0..4 {
        let mut seq = MadaraCmdBuilder::new()
            .label("parallel-merkle-disabled")
            .env([("RUST_LOG", "info")])
            .args(common_args(false))
            .run_no_wait();
        let mut par = MadaraCmdBuilder::new()
            .label("parallel-merkle-enabled")
            .env([("RUST_LOG", "info")])
            .args(common_args(true))
            .run_no_wait();

        seq.hook_stdout_and_wait_for_ports(true, false, false);
        par.hook_stdout_and_wait_for_ports(true, false, false);
        seq.wait_for_ready().await;
        par.wait_for_ready().await;
        seq.wait_for_sync_to(0).await;
        par.wait_for_sync_to(0).await;

        let seq_genesis_hash =
            extract_block_hash(seq.json_rpc().get_block_with_tx_hashes(BlockId::Number(0)).await.unwrap());
        let par_genesis_hash =
            extract_block_hash(par.json_rpc().get_block_with_tx_hashes(BlockId::Number(0)).await.unwrap());
        if seq_genesis_hash == par_genesis_hash {
            return (seq, par);
        }

        seq.stop();
        par.stop();
        sleep(Duration::from_millis(250)).await;
    }

    panic!("could not start sequential/parallel fresh nodes with matching genesis hash")
}

#[tokio::test(flavor = "multi_thread")]
async fn parallel_merkle_and_sequential_match_state_root_after_100_non_empty_blocks() {
    let sierra_class: SierraClass = serde_json::from_slice(m_cairo_test_contracts::TEST_CONTRACT_SIERRA).unwrap();
    let flattened_class = sierra_class.flatten().unwrap();
    let compiled_hashes = FlattenedSierraClass::from(flattened_class.clone()).compile_to_casm_with_hashes().unwrap();
    let compiled_class_hash = compiled_hashes.blake_hash;

    let (sequential, parallel) = start_synced_nodes().await;

    let seq_rpc = sequential.json_rpc();
    let par_rpc = parallel.json_rpc();

    let seq_chain_id = seq_rpc.chain_id().await.unwrap();
    let par_chain_id = par_rpc.chain_id().await.unwrap();
    assert_eq!(seq_chain_id, par_chain_id, "chain_id must match between sequential and parallel nodes");

    let mut seq_account = SingleOwnerAccount::new(
        seq_rpc.clone(),
        LocalWallet::from_signing_key(SigningKey::from_secret_scalar(ACCOUNT_SECRET)),
        ACCOUNT_ADDRESS,
        seq_chain_id,
        ExecutionEncoding::New,
    );
    let mut par_account = SingleOwnerAccount::new(
        par_rpc.clone(),
        LocalWallet::from_signing_key(SigningKey::from_secret_scalar(ACCOUNT_SECRET)),
        ACCOUNT_ADDRESS,
        par_chain_id,
        ExecutionEncoding::New,
    );
    seq_account.set_block_id(BlockId::Tag(BlockTag::Latest));
    par_account.set_block_id(BlockId::Tag(BlockTag::Latest));

    let seq_oz_class_hash = seq_rpc.get_class_hash_at(BlockId::Tag(BlockTag::Latest), ACCOUNT_ADDRESS).await.unwrap();
    let par_oz_class_hash = par_rpc.get_class_hash_at(BlockId::Tag(BlockTag::Latest), ACCOUNT_ADDRESS).await.unwrap();
    assert_eq!(
        seq_oz_class_hash, par_oz_class_hash,
        "OZ account class hash must match between sequential and parallel nodes"
    );

    let mut seq_account_factory = OpenZeppelinAccountFactory::new(
        seq_oz_class_hash,
        seq_chain_id,
        LocalWallet::from_signing_key(SigningKey::from_secret_scalar(DEPLOY_ACCOUNT_SECRET)),
        &seq_rpc,
    )
    .await
    .unwrap();
    let mut par_account_factory = OpenZeppelinAccountFactory::new(
        par_oz_class_hash,
        par_chain_id,
        LocalWallet::from_signing_key(SigningKey::from_secret_scalar(DEPLOY_ACCOUNT_SECRET)),
        &par_rpc,
    )
    .await
    .unwrap();
    seq_account_factory.set_block_id(BlockId::Tag(BlockTag::Latest));
    par_account_factory.set_block_id(BlockId::Tag(BlockTag::Latest));

    let seq_deploy = seq_account_factory.deploy_v3(DEPLOY_ACCOUNT_SALT);
    let par_deploy = par_account_factory.deploy_v3(DEPLOY_ACCOUNT_SALT);
    let deploy_address = seq_deploy.address();
    assert_eq!(deploy_address, par_deploy.address(), "deployed account address must match");

    let mut seq_nonce = seq_rpc.get_nonce(BlockId::Tag(BlockTag::Latest), ACCOUNT_ADDRESS).await.unwrap();
    let par_nonce = par_rpc.get_nonce(BlockId::Tag(BlockTag::Latest), ACCOUNT_ADDRESS).await.unwrap();
    assert_eq!(seq_nonce, par_nonce, "initial account nonce must match");

    let seq_funding =
        with_devnet_fees!(seq_account.execute_v3(vec![make_transfer_call(deploy_address, 1_000_000_000_000_000)]))
            .nonce(seq_nonce);
    let par_funding =
        with_devnet_fees!(par_account.execute_v3(vec![make_transfer_call(deploy_address, 1_000_000_000_000_000)]))
            .nonce(seq_nonce);
    let (funding_seq, funding_par) = tokio::join!(seq_funding.send(), par_funding.send());
    let funding_seq = funding_seq.unwrap();
    let funding_par = funding_par.unwrap();

    let (funding_seq_receipt, funding_par_receipt) = tokio::join!(
        wait_for_confirmed_receipt(&seq_rpc, funding_seq.transaction_hash),
        wait_for_confirmed_receipt(&par_rpc, funding_par.transaction_hash)
    );
    let (mut seq_last_tx_block, mut par_last_tx_block) =
        (funding_seq_receipt.block.block_number(), funding_par_receipt.block.block_number());
    seq_nonce += Felt::ONE;

    let seq_declare =
        with_devnet_fees!(seq_account.declare_v3(flattened_class.clone().into(), compiled_class_hash).nonce(seq_nonce));
    let par_declare =
        with_devnet_fees!(par_account.declare_v3(flattened_class.clone().into(), compiled_class_hash).nonce(seq_nonce));
    let (declare_seq, declare_par) = tokio::join!(seq_declare.send(), par_declare.send());
    let declare_seq = declare_seq.unwrap();
    let declare_par = declare_par.unwrap();
    let (declare_seq_receipt, declare_par_receipt) = tokio::join!(
        wait_for_confirmed_receipt(&seq_rpc, declare_seq.transaction_hash),
        wait_for_confirmed_receipt(&par_rpc, declare_par.transaction_hash)
    );
    seq_last_tx_block = seq_last_tx_block.max(declare_seq_receipt.block.block_number());
    par_last_tx_block = par_last_tx_block.max(declare_par_receipt.block.block_number());
    seq_nonce += Felt::ONE;

    let seq_deploy_tx = with_devnet_fees!(seq_deploy.nonce(Felt::ZERO));
    let par_deploy_tx = with_devnet_fees!(par_deploy.nonce(Felt::ZERO));
    let (deploy_seq, deploy_par) = tokio::join!(seq_deploy_tx.send(), par_deploy_tx.send());
    let deploy_seq = deploy_seq.unwrap();
    let deploy_par = deploy_par.unwrap();
    let (deploy_seq_receipt, deploy_par_receipt) = tokio::join!(
        wait_for_confirmed_receipt(&seq_rpc, deploy_seq.transaction_hash),
        wait_for_confirmed_receipt(&par_rpc, deploy_par.transaction_hash)
    );
    seq_last_tx_block = seq_last_tx_block.max(deploy_seq_receipt.block.block_number());
    par_last_tx_block = par_last_tx_block.max(deploy_par_receipt.block.block_number());

    for step in 4..=TX_COUNT_TARGET {
        let recipient = ACCOUNTS[(step % (ACCOUNTS.len() - 1)) + 1];
        let amount = 10 + step as u64;

        let seq_invoke =
            with_devnet_fees!(seq_account.execute_v3(vec![make_transfer_call(recipient, amount)]).nonce(seq_nonce));
        let par_invoke =
            with_devnet_fees!(par_account.execute_v3(vec![make_transfer_call(recipient, amount)]).nonce(seq_nonce));
        let (invoke_seq, invoke_par) = tokio::join!(seq_invoke.send(), par_invoke.send());
        let invoke_seq = invoke_seq.unwrap();
        let invoke_par = invoke_par.unwrap();
        let (invoke_seq_receipt, invoke_par_receipt) = tokio::join!(
            wait_for_confirmed_receipt(&seq_rpc, invoke_seq.transaction_hash),
            wait_for_confirmed_receipt(&par_rpc, invoke_par.transaction_hash)
        );

        seq_last_tx_block = seq_last_tx_block.max(invoke_seq_receipt.block.block_number());
        par_last_tx_block = par_last_tx_block.max(invoke_par_receipt.block.block_number());
        seq_nonce += Felt::ONE;
    }

    let comparison_block_n = MIN_BLOCK_HEIGHT_TARGET.max(seq_last_tx_block + 3).max(par_last_tx_block + 3);

    wait_for_block_at_least(&seq_rpc, comparison_block_n).await;
    wait_for_block_at_least(&par_rpc, comparison_block_n).await;

    let seq_final_root =
        extract_confirmed_root(seq_rpc.get_state_update(BlockId::Number(comparison_block_n)).await.unwrap());
    let par_final_root =
        extract_confirmed_root(par_rpc.get_state_update(BlockId::Number(comparison_block_n)).await.unwrap());

    assert_eq!(
        seq_final_root, par_final_root,
        "state roots diverged at block {}: sequential root {:#x}, parallel root {:#x}",
        comparison_block_n, seq_final_root, par_final_root
    );
}
