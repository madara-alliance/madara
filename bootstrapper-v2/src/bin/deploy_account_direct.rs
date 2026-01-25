use anyhow::Result;
use starknet::{
    accounts::{Account, ConnectedAccount, SingleOwnerAccount},
    core::types::{Felt, BlockId, BlockTag},
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider, Url},
    signers::{LocalWallet, SigningKey},
    core::utils::get_selector_from_name,
};

const RPC_URL: &str = "http://localhost:9944";
const BOOTSTRAP_PRIVATE_KEY: &str = "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
const BOOTSTRAP_ADDRESS: &str = "0x0492a34ada7acb0dda8636b9d36a22cd4f6bb50b6421f1bd64d098de60835f9b";

// OpenZeppelin Account class hash
const OZ_ACCOUNT_CLASS_HASH: &str = "0x00e81f6009f96661c969f14c40d8b453cc40fc6c674607a61c23bb3563709e2a";

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let public_key_hex = std::env::args()
        .nth(1)
        .expect("Usage: deploy_account_direct <public_key>");

    println!("\n🚀 Deploying OpenZeppelin Account (Direct Method)...");
    println!("Public key: {}", public_key_hex);

    // Setup provider
    let provider = JsonRpcClient::new(HttpTransport::new(Url::parse(RPC_URL)?));
    let chain_id = provider.chain_id().await?;

    // Setup bootstrap account (signer)
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
    account.set_block_id(BlockId::Tag(BlockTag::Latest));

    // Parse inputs
    let public_key = Felt::from_hex(&public_key_hex)?;
    let class_hash = Felt::from_hex(OZ_ACCOUNT_CLASS_HASH)?;

    // Use a different salt to get a different address
    let salt = Felt::from_hex("0x1234")?;

    println!("\n📝 Deploying account via deploy_contract...");
    println!("Class hash: {:#066x}", class_hash);
    println!("Salt: {:#066x}", salt);

    // Try using deploy_contract from account
    let deploy_result = account
        .deploy_contract(
            class_hash,
            vec![public_key], // constructor calldata
            salt,
        )
        .send()
        .await?;

    println!("Deploy transaction hash: {:#066x}", deploy_result.transaction_hash);
    println!("Deployed address: {:#066x}", deploy_result.contract_address);

    // Wait for transaction
    println!("\n⏳ Waiting for deployment confirmation...");
    loop {
        match provider.get_transaction_receipt(deploy_result.transaction_hash).await {
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
    println!("Address: {:#066x}", deploy_result.contract_address);
    println!("\nNow transfer ETH from the old address to this new address:");
    println!("Old address: 0x06d969402aec588361e93df9cb076f03c037923b644fd3f863818f38e6c94ccd");
    println!("New address: {:#066x}", deploy_result.contract_address);

    Ok(())
}
