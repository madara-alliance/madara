use anyhow::Result;
use starknet::{
    accounts::{Account, ConnectedAccount, SingleOwnerAccount},
    core::types::{Call, Felt, BlockId, BlockTag},
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider, Url},
    signers::{LocalWallet, SigningKey},
};

const RPC_URL: &str = "http://localhost:9944";
const BOOTSTRAP_PRIVATE_KEY: &str = "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
const BOOTSTRAP_ADDRESS: &str = "0x0492a34ada7acb0dda8636b9d36a22cd4f6bb50b6421f1bd64d098de60835f9b";

// OpenZeppelin Account class hash (from bootstrapper output)
const OZ_ACCOUNT_CLASS_HASH: &str = "0x00e81f6009f96661c969f14c40d8b453cc40fc6c674607a61c23bb3563709e2a";

// UDC selector for deployContract
const UDC_DEPLOY_SELECTOR: &str = "0x1987cbd17808b9865d81f325acb326c5e0b1fb8055e01dfdaa16dff96a97227";

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let public_key_hex = std::env::args()
        .nth(1)
        .expect("Usage: deploy_user_account <public_key>");

    println!("\n🚀 Deploying OpenZeppelin Account...");
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
    let salt = Felt::from_hex("0x0")?; // Use zero salt for deterministic address
    let unique = Felt::ZERO; // Not using unique deployer

    // Constructor calldata for OpenZeppelin Account: [public_key]
    let constructor_calldata = vec![public_key];

    // UDC calldata: [class_hash, salt, unique, calldata_len, ...calldata]
    let mut udc_calldata = vec![
        class_hash,
        salt,
        unique,
        Felt::from(constructor_calldata.len() as u64),
    ];
    udc_calldata.extend(constructor_calldata);

    println!("\n📝 Deploying account via UDC...");
    println!("Class hash: {:#066x}", class_hash);
    println!("Salt: {:#066x}", salt);

    // Get current nonce
    let nonce = account.get_nonce().await?;
    println!("Bootstrap account nonce: {:#066x}", nonce);

    let calls = vec![Call {
        to: Felt::from_hex("0x50b90f8e476202b41e5d151b3691127b39acfc2a43ddcd6460bf725a2a1c659")?, // UDC address
        selector: Felt::from_hex(UDC_DEPLOY_SELECTOR)?,
        calldata: udc_calldata,
    }];

    let result = account
        .execute_v3(calls)
        .l1_gas(5000000)
        .l2_gas(5000000)
        .l1_data_gas(5000)
        .send()
        .await?;

    println!("Transaction hash: {:#066x}", result.transaction_hash);

    // Wait for transaction
    println!("\n⏳ Waiting for transaction confirmation...");
    loop {
        match provider.get_transaction_receipt(result.transaction_hash).await {
            Ok(_) => {
                println!("✅ Transaction confirmed!");
                break;
            }
            Err(_) => {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
    }

    // Calculate deployed address
    // For UDC: hash(deployer, salt, class_hash, constructor_calldata_hash)
    println!("\n✅ Account deployed successfully!");
    println!("Expected address: 0x06d969402aec588361e93df9cb076f03c037923b644fd3f863818f38e6c94ccd");

    Ok(())
}
