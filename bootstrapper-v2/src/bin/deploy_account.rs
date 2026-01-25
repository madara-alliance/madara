use anyhow::Result;
use starknet::{
    accounts::{Account, AccountFactory, ExecutionEncoding, OpenZeppelinAccountFactory, SingleOwnerAccount},
    core::types::{contract::SierraClass, BlockId, BlockTag, Call, Felt},
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider, Url},
    signers::{LocalWallet, SigningKey, Signer},
};
use std::sync::Arc;

const ACCOUNT_CLASS_HASH: &str = "0x00e81f6009f96661c969f14c40d8b453cc40fc6c674607a61c23bb3563709e2a";
const RPC_URL: &str = "http://localhost:9944";

// Pre-deployed Madara account to fund the new account
const FUNDER_ADDRESS: &str = "0x055be462e718c4166d656d11f89e341115b8bc82389c3762a10eade04fcb225d";
const FUNDER_PRIVATE_KEY: &str = "0x077e56c6dc32d40a67f6f7e6625c8dc5e570abe49c0a24e9202e4ae906abcc07";

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // Generate a new random private key
    let private_key_scalar = SigningKey::from_random();
    let private_key = private_key_scalar.secret_scalar();
    let private_key_hex = format!("{:#066x}", private_key);

    println!("\n🔑 Generated new account credentials:");
    println!("Private Key: {}", private_key_hex);

    // Create signer and get public key
    let signer = LocalWallet::from(private_key_scalar);
    let public_key = signer.get_public_key().await?;
    println!("Public Key:  {:#066x}", public_key.scalar());

    // Setup provider
    let provider = JsonRpcClient::new(HttpTransport::new(Url::parse(RPC_URL).expect("Invalid RPC URL")));
    let chain_id = provider.chain_id().await?;

    // Parse account class hash
    let class_hash = Felt::from_hex(ACCOUNT_CLASS_HASH)?;

    // Generate a random salt for deployment
    let salt = Felt::from_hex_unchecked(&format!("{:#066x}", rand::random::<u128>()));

    println!("\n📦 Deploying account contract...");

    // Create account factory
    let factory = OpenZeppelinAccountFactory::new(class_hash, chain_id, &signer, &provider).await?;

    // Deploy account with sufficient gas
    let deployment = factory
        .deploy_v3(salt)
        .l1_gas(5000000)
        .l2_gas(5000000)
        .l1_data_gas(5000);

    let deploy_result = deployment.send().await?;

    println!("Account Address: {:#066x}", deploy_result.contract_address);
    println!("Transaction Hash: {:#066x}", deploy_result.transaction_hash);

    // Wait for deployment
    wait_for_tx(&provider, deploy_result.transaction_hash).await?;

    println!("\n✅ Account deployed successfully!");
    println!("\n📋 Account Summary:");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("Address:     {:#066x}", deploy_result.contract_address);
    println!("Private Key: {}", private_key_hex);
    println!("Public Key:  {:#066x}", public_key.scalar());
    println!("Class Hash:  {}", ACCOUNT_CLASS_HASH);
    println!("Salt:        {:#066x}", salt);
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("\n💡 To fund this account, transfer tokens from a pre-deployed account:");
    println!("   Funder: {}", FUNDER_ADDRESS);

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
