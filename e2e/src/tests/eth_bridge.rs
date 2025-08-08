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

// Define the StarkNet ETH Bridge contract interface
sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract StarknetEthBridge {
        /// Main deposit function - inherited from LegacyBridge
        /// @param amount The amount of ETH to deposit (in wei)
        /// @param l2Recipient The StarkNet address to receive the funds (as uint256)
        function deposit(uint256 amount, uint256 l2Recipient) external payable;
    }
}

// Taken from : addresses.json, bootstrapper.json and output of bootstrapper

const L2_CHAIN_ID_STRING: &str = "MADARA_DEVNET"; // Madara Chain ID
const L2_ACCOUNT_ADDRESS: &str = "0x4fe5eea46caa0a1f344fafce82b39d66b552f00d3cd12e89073ef4b4ab37860";
const L2_ACCOUNT_PRIVATE_KEY: &str = "0xabcd"; // Hex Madara Account Private Key

const L2_ETH_TOKEN_ADDRESS: &str = "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7"; // Hex Madara ETH TOKEN Address
// const L2_ETH_BRIDGE_ADDRESS: &str = "0xabc"; // Hex Madara ETH BRIDGE Address

const L1_ACCOUNT_ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"; // Hex L1 Account Address
const L1_ACCOUNT_PRIVATE_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"; // Hex L1 Account Private Key

const L1_ETH_BRIDGE_ADDRESS: &str = "0x8a791620dd6260079bf849dc5567adc3f2fdc318"; // Hex L1 ETH BRIDGE Address


#[allow(unused_imports)]
use super::setup::setup_chain;
#[allow(unused_imports)]
use crate::setup::ChainSetup;

#[rstest]
#[case("ethbridgedeposit")]
#[tokio::test]
async fn test_eth_bridge_deposit(
    #[case] test_name: &str,
    #[future]
    #[with(test_name)]
    setup_chain: ChainSetup,
) {
    // Ensuring setup stays in scope
    let setup = setup_chain.await;

    let test_config = setup.config().clone();

    // Deposit ETH bridge test here !
    // L1 -> L2

    // STEP 1 : Getting the Account's ETH Token Balance before Deposit
        let l2_provider = JsonRpcClient::new(HttpTransport::new(
                Url::parse(test_config.get_madara_config().rpc_endpoint().as_str()).unwrap(),
        ));

        let l2_address = Felt::from_hex(L2_ACCOUNT_ADDRESS).unwrap();

        let l2_eth_token_address = Felt::from_str(L2_ETH_TOKEN_ADDRESS).unwrap();

        let l2_account_eth_balance_before_deposit = l2_read_token_balance(
            &l2_provider,
            l2_eth_token_address,
            l2_address,
        ).await[0];

    // STEP 2 : Get the L1 provider, account, eth bridge address and do deposit

        let l1_signer = PrivateKeySigner::from_str(L1_ACCOUNT_PRIVATE_KEY).unwrap();
        let l1_wallet = EthereumWallet::from(l1_signer.clone());

        let l1_provider = ProviderBuilder::new()
            .with_recommended_fillers()
            .wallet(l1_wallet)
            .on_http(test_config.get_anvil_config().endpoint());

        let l1_bridge_contract = StarknetEthBridge::new(L1_ETH_BRIDGE_ADDRESS.parse().unwrap(), &l1_provider);

        let l2_recipient = U256::from_str(l2_address.to_hex_string().as_str()).unwrap();
        let deposit_amount = U256::from(100_000_000_000_000_000u64); // 0.1 ETH
        let fee_amount = U256::from(10_000_000_000_000_000u64); // 0.01 ETH fee
        let total_value = deposit_amount + fee_amount; // Total ETH to send

        let txn = l1_bridge_contract
            .deposit(deposit_amount, l2_recipient)
            .value(total_value) // msg.value = deposit + fee
            .send()
            .await.unwrap();

        let txn_receipt = txn.with_required_confirmations(3).get_receipt().await.unwrap();
        let deposit_txn_hash = txn_receipt.transaction_hash.to_string();
        println!("Deposit Transaction Hash : {:?}", deposit_txn_hash);

    // STEP 3 : Wait for a pre-defined time, which ensures that by that time Madara must have processed the L1 Message
        sleep(Duration::from_secs(40)).await;

    // STEP 4 : Getting the Account's ETH Token Balance after Deposit

        // TODO: sleep for atleast 3 blocks ?
        let l2_account_eth_balance_after_deposit = l2_read_token_balance(
            &l2_provider,
            l2_eth_token_address,
            l2_address,
        ).await[0];


    // STEP 5 : balance after > balance before

        println!("Balance before deposit: {}", l2_account_eth_balance_before_deposit);
        println!("Balance after deposit: {}", l2_account_eth_balance_after_deposit);
        assert!(l2_account_eth_balance_after_deposit > l2_account_eth_balance_before_deposit,
            "Balance after deposit should be greater than balance before deposit");
        println!("ETH Deposit successful");


    // STEP 6 :Delete the created directory
        let dir_path = get_file_path(test_name);
        if let Err(err) = std::fs::remove_dir_all(&dir_path) {
            eprintln!("Failed to delete directory: {}", err);
        }


}




// async fn deposit_eth_extra(test_config: SetupConfig) -> color_eyre::Result<()> {

//     let bootstrapper_config_reader = JsonReader::new(get_file_path(BOOTSTRAPPER_CONFIG)).unwrap();

//     // Deposit ETH bridge test here !

//     // STEP 1 : Create the L2 Provider, Account, ETH Token & Bridge
//         let l2_provider = JsonRpcClient::new(HttpTransport::new(
//                 Url::parse(test_config.get_madara_config().rpc_endpoint().as_str()).unwrap(),
//         ));

//         let l2_signer = LocalWallet::from(SigningKey::from_secret_scalar(
//                 Felt::from_hex(L2_ACCOUNT_PRIVATE_KEY).unwrap(),
//         ));

//         let l2_address = Felt::from_hex(L2_ACCOUNT_ADDRESS).unwrap();

//         let l2_account = SingleOwnerAccount::new(
//             &l2_provider,
//             l2_signer,
//             l2_address,
//             Felt::from_str(L2_CHAIN_ID_STRING).unwrap(),
//             ExecutionEncoding::New,
//         );

//         let l2_eth_token_address = Felt::from_str(L2_ETH_TOKEN_ADDRESS).unwrap();

//         let l2_eth_bridge_address = Felt::from_str(L2_ETH_BRIDGE_ADDRESS).unwrap();


//     // STEP 2 : Getting the Account's ETH Token Balance before Deposit

//         let l2_account_eth_balance_before_deposit = l2_read_token_balance(
//             &l2_provider,
//             l2_eth_token_address,
//             l2_address,
//         ).await;


//     // STEP 3 : Get the L1 provider, account, eth bridge address and do deposit

//         let l1_provider = ProviderBuilder::new()
//             .on_http(L1_RPC_URL.parse().unwrap());
//         let l1_signer = PrivateKeySigner::from_str(L1_ACCOUNT_PRIVATE_KEY).unwrap();
//         let l1_wallet = EthereumWallet::from(l1_signer.clone());

//         let bridge_contract = StarknetEthBridge::new(L1_ETH_BRIDGE_ADDRESS.parse().unwrap(), &l1_provider);

//         let l2_recipient = U256::from_str(l2_address.to_hex_string().as_str()).unwrap();
//         let deposit_amount = U256::from(100_000_000_000_000_000u64); // 0.1 ETH
//         let fee_amount = U256::from(10_000_000_000_000_000u64); // 0.01 ETH fee
//         let total_value = deposit_amount + fee_amount; // Total ETH to send

//         let txn = bridge_contract
//             .deposit(deposit_amount, l2_recipient)
//             .value(total_value) // msg.value = deposit + fee
//             .send()
//             .await.unwrap();

//         let txn_receipt = txn.with_required_confirmations(3).get_receipt().await.unwrap();
//         let deposit_txn_hash = txn_receipt.transaction_hash.to_string();


//     // STEP 4 : Getting the Account's ETH Token Balance after Deposit

//         // TODO: sleep for atleast 3 blocks ?
//         let l2_account_eth_balance_after_deposit = l2_read_token_balance(
//             &l2_provider,
//             l2_eth_token_address,
//             l2_address,
//         ).await;


//     // STEP 5 : balance after > balance before
//     assert!(l2_account_eth_balance_after_deposit > l2_account_eth_balance_before_deposit,
//         "Balance after deposit should be greater than balance before deposit");
//     println!("ETH Deposit successful");

//     Ok(())
// }


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
