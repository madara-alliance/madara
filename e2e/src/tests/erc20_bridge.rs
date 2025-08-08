use alloy::primitives::Address;
use rstest::*;
use starknet_signers::{LocalWallet, SigningKey};
use tokio::time::sleep;

use std::str::FromStr;
use std::time::Duration;

use alloy::network::EthereumWallet;
use alloy::providers::ProviderBuilder;
use alloy::signers::local::PrivateKeySigner;
use crate::services::constants::*;
use crate::services::helpers::get_database_path;
use crate::services::helpers::get_file_path;
use crate::setup::SetupConfig;
use starknet_core::types::FunctionCall;
use starknet_core::types::BlockTag;
use starknet::providers::Provider;
use starknet_core::types::BlockId;
use starknet_core::utils::get_selector_from_name;

use alloy::{
    primitives::U256,
    sol,
};

use starknet::{
    accounts::{ExecutionEncoding, SingleOwnerAccount},
    core::types::Felt,
    providers::{
        jsonrpc::{HttpTransport, JsonRpcClient},
        Url,
    },
};

use crate::tests::setup::JsonReader;

// Define the ERC20Token contract interface
sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract ERC20Token {
        function approve(address spender, uint256 amount) external returns (bool);
    }
}

// Define the StarkNet ERC20 Bridge contract interface
sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract StarknetTokenBridge {
        /// Main deposit function - inherited from LegacyBridge
        /// @param token The address of the token to deposit
        /// @param amount The amount of tokens to deposit
        /// @param l2Recipient The StarkNet address to receive the funds (as uint256)
        function deposit(address token, uint256 amount, uint256 l2Recipient) external payable;
    }
}

// Taken from : addresses.json, bootstrapper.json and output of bootstrapper

const L2_CHAIN_ID_STRING: &str = "MADARA_DEVNET"; // Madara Chain ID
const L2_ACCOUNT_ADDRESS: &str = "0x4fe5eea46caa0a1f344fafce82b39d66b552f00d3cd12e89073ef4b4ab37860";
const L2_ACCOUNT_PRIVATE_KEY: &str = "0xabcd"; // Hex Madara Account Private Key

const L2_ERC20_TOKEN_ADDRESS: &str = "0x25205e11d1c0017f94a531a139b47137dae34ae0b9bed9e8fe698ace64f0609"; // Hex Madara ERC20 TOKEN Address
const L2_ERC20_BRIDGE_ADDRESS: &str = "0x7e46129030dcff37062dd4353a738a7a5d5a88e8481bfa8b36a2ef5f8f7fa47"; // Hex Madara ERC20 BRIDGE Address

const L1_ACCOUNT_ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"; // Hex L1 Account Address
const L1_ACCOUNT_PRIVATE_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"; // Hex L1 Account Private Key

const L1_ERC20_BRIDGE_ADDRESS: &str = "0x59b670e9fa9d0a427751af201d676719a970857b"; // Hex L1 ERC20 BRIDGE Address
const L1_ERC20_TOKEN_ADDRESS: &str = "0x4ed7c70f96b99c776995fb64377f0d4ab3b0e1c1";

#[allow(unused_imports)]
use super::setup::setup_chain;
#[allow(unused_imports)]
use crate::setup::ChainSetup;

#[rstest]
#[case("erc20bridgedeposit")]
#[tokio::test]
async fn test_erc20_bridge_deposit(
    #[case] test_name: &str,
    #[future]
    #[with(test_name)]
    setup_chain: ChainSetup,
) {
    // Ensuring setup stays in scope
    let setup = setup_chain.await;

    let test_config = setup.config().clone();

    // Deposit ERC20 bridge test here !
    // L1 -> L2

    // STEP 1 : Getting the Account's ERC20 Token Balance before Deposit
        let l2_provider = JsonRpcClient::new(HttpTransport::new(
                Url::parse(test_config.get_madara_config().rpc_endpoint().as_str()).unwrap(),
        ));

        let l2_address = Felt::from_hex(L2_ACCOUNT_ADDRESS).unwrap();

        let l2_erc20_token_address = Felt::from_str(L2_ERC20_TOKEN_ADDRESS).unwrap();

        let l2_account_erc20_balance_before_deposit = l2_read_token_balance(
            &l2_provider,
            l2_erc20_token_address,
            l2_address,
        ).await[0];

    // STEP 2 : Get the L1 provider, account, ERC20 bridge address and do deposit

        let l1_signer = PrivateKeySigner::from_str(L1_ACCOUNT_PRIVATE_KEY).unwrap();
        let l1_wallet = EthereumWallet::from(l1_signer.clone());

        let l1_provider = ProviderBuilder::new()
            .with_recommended_fillers()
            .wallet(l1_wallet)
            .on_http(test_config.get_anvil_config().endpoint());

        let l1_erc20_token_address =  Address::from_str(L1_ERC20_TOKEN_ADDRESS).unwrap();
        let l1_erc20_bridge_address = Address::from_str(L1_ERC20_BRIDGE_ADDRESS).unwrap();
        let l1_erc20_token = ERC20Token::new(l1_erc20_token_address, &l1_provider);
        let l1_bridge_contract = StarknetTokenBridge::new(l1_erc20_bridge_address, &l1_provider);

        let l2_recipient = U256::from_str(L2_ACCOUNT_ADDRESS).unwrap();
        let deposit_amount = U256::from(10u64); //
        let fee_amount = U256::from(10_000_000_000_000_000u64); // 0.01 ETH fee

        // APPROVE and DEPOSIT

        let approve_txn = l1_erc20_token
            .approve(l1_erc20_bridge_address, deposit_amount)
            .send()
            .await.unwrap();

        let approve_txn_receipt = approve_txn.with_required_confirmations(3).get_receipt().await.unwrap();
        let approve_txn_hash = approve_txn_receipt.transaction_hash.to_string();
        println!("Approve Transaction Hash : {:?}", approve_txn_hash);


        let txn = l1_bridge_contract
            .deposit(l1_erc20_token_address, deposit_amount, l2_recipient)
            .value(fee_amount) // msg.value = deposit + fee
            .send()
            .await.unwrap();

        let txn_receipt = txn.with_required_confirmations(3).get_receipt().await.unwrap();
        let deposit_txn_hash = txn_receipt.transaction_hash.to_string();
        println!("Deposit Transaction Hash : {:?}", deposit_txn_hash);

    // STEP 3 : Wait for a pre-defined time, which ensures that by that time Madara must have processed the L1 Message
        sleep(Duration::from_secs(40)).await;

    // STEP 4 : Getting the Account's ERC20 Token Balance after Deposit

        // TODO: sleep for atleast 3 blocks ?
        let l2_account_erc20_balance_after_deposit = l2_read_token_balance(
            &l2_provider,
            l2_erc20_token_address,
            l2_address,
        ).await[0];


    // STEP 5 : balance after > balance before

        println!("Balance before deposit: {}", l2_account_erc20_balance_before_deposit);
        println!("Balance after deposit: {}", l2_account_erc20_balance_after_deposit);
        assert!(l2_account_erc20_balance_after_deposit > l2_account_erc20_balance_before_deposit,
            "Balance after deposit should be greater than balance before deposit");
        println!("ERC20 Deposit successful");


    // STEP 6 :Delete the created directory
        let dir_path = get_file_path(test_name);
        if let Err(err) = std::fs::remove_dir_all(&dir_path) {
            eprintln!("Failed to delete directory: {}", err);
        }


}





pub async fn l2_read_token_balance(
    rpc: &JsonRpcClient<HttpTransport>,
    contract_address: Felt,
    account_address: Felt,
) -> Vec<Felt> {
    rpc.call(
        FunctionCall {
            contract_address,
            entry_point_selector: get_selector_from_name("balanceOf").unwrap(),
            calldata: vec![account_address],
        },
        BlockId::Tag(BlockTag::Latest),
    )
    .await
    .unwrap()
}
