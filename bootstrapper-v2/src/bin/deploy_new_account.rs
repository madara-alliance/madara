use anyhow::Result;
use starknet::{
    accounts::{AccountFactory, OpenZeppelinAccountFactory, SingleOwnerAccount},
    core::types::{Felt, BlockId, BlockTag},
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider, Url},
    signers::{LocalWallet, SigningKey},
};

const RPC_URL: &str = "http://localhost:9944";

// OpenZeppelin Account class hash
const OZ_ACCOUNT_CLASS_HASH: &str = "0x00e81f6009f96661c969f14c40d8b453cc40fc6c674607a61c23bb3563709e2a";

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let private_key_hex = std::env::args()
        .nth(1)
        .expect("Usage: deploy_new_account <private_key>");

    println!("\n🚀 Deploying New OpenZeppelin Account...");
    println!("Private key: {}", private_key_hex);

    // Setup provider
    let provider = JsonRpcClient::new(HttpTransport::new(Url::parse(RPC_URL)?));
    let chain_id = provider.chain_id().await?;

    // Parse private key and derive public key
    let private_key = SigningKey::from_secret_scalar(Felt::from_hex(&private_key_hex)?);
    let public_key = private_key.verifying_key().scalar();

    println!("Public key: {:#066x}", public_key);

    // Create signer
    let signer = LocalWallet::from(private_key);

    // Create account factory
    let class_hash = Felt::from_hex(OZ_ACCOUNT_CLASS_HASH)?;
    let factory = OpenZeppelinAccountFactory::new(
        class_hash,
        chain_id,
        signer,
        &provider,
    ).await?;

    // Use a specific salt to get deterministic address
    let salt = Felt::from_hex("0x5678")?;

    println!("\n📝 Deploying account...");
    println!("Class hash: {:#066x}", class_hash);
    println!("Salt: {:#066x}", salt);

    // Deploy the account
    let deployment = factory.deploy_v3(salt);

    // Pre-compute the address
    let address = deployment.address();
    println!("Pre-computed address: {:#066x}", address);

    // Send the deployment transaction with high gas limits
    let result = deployment
        .l1_gas(100000000)
        .l2_gas(100000000)
        .l1_data_gas(100000)
        .send()
        .await?;

    println!("\nDeploy transaction hash: {:#066x}", result.transaction_hash);

    // Wait for transaction
    println!("\n⏳ Waiting for deployment confirmation...");
    loop {
        match provider.get_transaction_receipt(result.transaction_hash).await {
            Ok(_) => {
                println!("✅ Deployment confirmed!");
                break;
            }
            Err(_) => {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
    }

    println!("\n✅ Account deployed successfully!");
    println!("Address: {:#066x}", address);
    println!("Private key: {}", private_key_hex);
    println!("Public key: {:#066x}", public_key);

    Ok(())
}
