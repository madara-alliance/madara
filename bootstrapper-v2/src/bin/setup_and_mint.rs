use anyhow::Result;
use starknet::{
    accounts::{Account, SingleOwnerAccount},
    core::types::{Call, Felt},
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider, Url},
    signers::{LocalWallet, SigningKey},
};

const RPC_URL: &str = "http://localhost:9944";
const BOOTSTRAP_PRIVATE_KEY: &str = "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
const BOOTSTRAP_ADDRESS: &str = "0x0492a34ada7acb0dda8636b9d36a22cd4f6bb50b6421f1bd64d098de60835f9b";

// Selector for register_app_role_admin
const REGISTER_APP_ROLE_ADMIN_SELECTOR: &str = "0x22184aba599752c4c1ccb086e1e5ffdf8ad0f47f2d6be3e85b7c40f19b89a83";
// Selector for register_app_governor
const REGISTER_APP_GOVERNOR_SELECTOR: &str = "0x33e4d57b18c7d1e8844e4762c0d92e120779c4a22f8e869b7f32c85c5549a66";
// Selector for mint
const MINT_SELECTOR: &str = "0x151e58b29c3f7a0b88849654091d3f19f8d7b00e295f868eeaf13e8b96d4d9e";

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let l2_eth_token = std::env::args()
        .nth(1)
        .expect("Usage: setup_and_mint <l2_eth_token_address> <recipient_address> <amount_low> <amount_high>");
    let recipient_address = std::env::args()
        .nth(2)
        .expect("Usage: setup_and_mint <l2_eth_token_address> <recipient_address> <amount_low> <amount_high>");
    let amount_low = std::env::args()
        .nth(3)
        .expect("Usage: setup_and_mint <l2_eth_token_address> <recipient_address> <amount_low> <amount_high>");
    let amount_high = std::env::args()
        .nth(4)
        .expect("Usage: setup_and_mint <l2_eth_token_address> <recipient_address> <amount_low> <amount_high>");

    println!("\n🔧 Setting up L2 ETH token roles and minting...");
    println!("L2 ETH Token: {}", l2_eth_token);
    println!("Recipient: {}", recipient_address);
    println!("Amount low: {}, high: {}", amount_low, amount_high);

    // Setup provider
    let provider = JsonRpcClient::new(HttpTransport::new(Url::parse(RPC_URL)?));
    let chain_id = provider.chain_id().await?;

    // Setup signer and account
    let private_key = SigningKey::from_secret_scalar(Felt::from_hex(BOOTSTRAP_PRIVATE_KEY)?);
    let signer = LocalWallet::from(private_key);
    let address = Felt::from_hex(BOOTSTRAP_ADDRESS)?;

    let mut account = SingleOwnerAccount::new(
        provider.clone(),
        signer,
        address,
        chain_id,
        starknet::accounts::ExecutionEncoding::New,
    );
    account.set_block_id(starknet::core::types::BlockId::Tag(starknet::core::types::BlockTag::Latest));

    let token_address = Felt::from_hex(&l2_eth_token)?;
    let recipient = Felt::from_hex(&recipient_address)?;
    let amt_low = Felt::from_hex(&amount_low)?;
    let amt_high = Felt::from_hex(&amount_high)?;

    println!("\n📝 Step 1: Granting APP_ROLE_ADMIN to bootstrap account...");
    let grant_app_role_admin = vec![Call {
        to: token_address,
        selector: Felt::from_hex(REGISTER_APP_ROLE_ADMIN_SELECTOR)?,
        calldata: vec![address],
    }];

    let result1 = account
        .execute_v3(grant_app_role_admin)
        .l1_gas(5000000)
        .l2_gas(5000000)
        .l1_data_gas(5000)
        .send()
        .await?;

    println!("Transaction hash: {:#066x}", result1.transaction_hash);
    wait_for_tx(&provider, result1.transaction_hash).await?;

    println!("\n📝 Step 2: Granting APP_GOVERNOR to bootstrap account...");
    let grant_app_governor = vec![Call {
        to: token_address,
        selector: Felt::from_hex(REGISTER_APP_GOVERNOR_SELECTOR)?,
        calldata: vec![address],
    }];

    let result2 = account
        .execute_v3(grant_app_governor)
        .l1_gas(5000000)
        .l2_gas(5000000)
        .l1_data_gas(5000)
        .send()
        .await?;

    println!("Transaction hash: {:#066x}", result2.transaction_hash);
    wait_for_tx(&provider, result2.transaction_hash).await?;

    println!("\n📝 Step 3: Minting tokens...");
    let mint_call = vec![Call {
        to: token_address,
        selector: Felt::from_hex(MINT_SELECTOR)?,
        calldata: vec![recipient, amt_low, amt_high],
    }];

    let result3 = account
        .execute_v3(mint_call)
        .l1_gas(5000000)
        .l2_gas(5000000)
        .l1_data_gas(5000)
        .send()
        .await?;

    println!("Transaction hash: {:#066x}", result3.transaction_hash);
    wait_for_tx(&provider, result3.transaction_hash).await?;

    println!("\n✅ Successfully set up roles and minted tokens!");

    Ok(())
}

async fn wait_for_tx(provider: &JsonRpcClient<HttpTransport>, tx_hash: Felt) -> Result<()> {
    use std::time::Duration;
    use tokio::time::sleep;

    for _ in 0..20 {
        if provider.get_transaction_receipt(tx_hash).await.is_ok() {
            return Ok(());
        }
        sleep(Duration::from_millis(500)).await;
    }

    anyhow::bail!("Transaction timeout waiting for {:#066x}", tx_hash)
}
