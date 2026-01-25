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

// Selector for transfer
const TRANSFER_SELECTOR: &str = "0x83afd3f4caedc6eebf44246fe54e38c95e3179a5ec9ea81740eca5b482d12e";

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let l2_eth_token = std::env::args()
        .nth(1)
        .expect("Usage: transfer_eth <l2_eth_token> <recipient> <amount_low> <amount_high>");
    let recipient_address = std::env::args()
        .nth(2)
        .expect("Usage: transfer_eth <l2_eth_token> <recipient> <amount_low> <amount_high>");
    let amount_low = std::env::args()
        .nth(3)
        .expect("Usage: transfer_eth <l2_eth_token> <recipient> <amount_low> <amount_high>");
    let amount_high = std::env::args()
        .nth(4)
        .expect("Usage: transfer_eth <l2_eth_token> <recipient> <amount_low> <amount_high>");

    println!("\n💸 Transferring ETH tokens...");
    println!("Token: {}", l2_eth_token);
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

    println!("\n📝 Executing transfer transaction...");

    let calls = vec![Call {
        to: token_address,
        selector: Felt::from_hex(TRANSFER_SELECTOR)?,
        calldata: vec![recipient, amt_low, amt_high],
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

    println!("\n✅ Successfully transferred ETH tokens to {}", recipient_address);

    Ok(())
}
