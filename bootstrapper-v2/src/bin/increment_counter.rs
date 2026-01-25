use anyhow::Result;
use starknet::{
    accounts::{Account, ConnectedAccount, SingleOwnerAccount},
    core::types::{Call, Felt, BlockId, BlockTag},
    core::utils::get_selector_from_name,
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider, Url},
    signers::{LocalWallet, SigningKey},
};

const RPC_URL: &str = "http://localhost:9944";
const BOOTSTRAP_PRIVATE_KEY: &str = "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
const BOOTSTRAP_ADDRESS: &str = "0x0492a34ada7acb0dda8636b9d36a22cd4f6bb50b6421f1bd64d098de60835f9b";
const SIMPLE_COUNTER_ADDRESS: &str = "0x033563a251fbc67635a890f5221ec646715496acab714f2409da9aee73c062c0";

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    println!("\n🔢 Incrementing SimpleCounter...");
    println!("Contract: {}", SIMPLE_COUNTER_ADDRESS);

    // Setup provider
    let provider = JsonRpcClient::new(HttpTransport::new(Url::parse(RPC_URL)?));
    let chain_id = provider.chain_id().await?;

    // Setup bootstrap account
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

    // Read counter value before increment
    println!("\n📖 Reading counter before increment...");
    let counter_address = Felt::from_hex(SIMPLE_COUNTER_ADDRESS)?;
    let get_counter_selector = get_selector_from_name("get_counter")?;

    let result = provider.call(
        starknet::core::types::FunctionCall {
            contract_address: counter_address,
            entry_point_selector: get_counter_selector,
            calldata: vec![],
        },
        BlockId::Tag(BlockTag::Latest),
    ).await?;

    println!("Counter value before: {}", result[0]);

    // Call increment
    println!("\n📝 Calling increment()...");
    let increment_selector = get_selector_from_name("increment")?;

    let calls = vec![Call {
        to: counter_address,
        selector: increment_selector,
        calldata: vec![],
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

    // Read counter value after increment
    println!("\n📖 Reading counter after increment...");
    let result = provider.call(
        starknet::core::types::FunctionCall {
            contract_address: counter_address,
            entry_point_selector: get_counter_selector,
            calldata: vec![],
        },
        BlockId::Tag(BlockTag::Latest),
    ).await?;

    println!("Counter value after: {}", result[0]);

    println!("\n✅ Successfully incremented counter!");

    Ok(())
}
