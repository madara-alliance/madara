use starknet::{
    accounts::{Account, ConnectedAccount, ExecutionEncoding, SingleOwnerAccount},
    core::types::{Call, Felt},
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider, Url},
    signers::{LocalWallet, SigningKey},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse("http://localhost:9944").unwrap(),
    ));

    let signer = LocalWallet::from(SigningKey::from_secret_scalar(
        Felt::from_hex("0x063bb0bb54374247449a9f6f978a3653b969e2a1294b35205d721ebc1539446a").unwrap(),
    ));

    let account_address = Felt::from_hex("0x012aa6059457fc2d02240962a6573e051fa919632853e6ba70207d7cef6be4c3").unwrap();
    let chain_id = Felt::from_hex("0x4d4144415241").unwrap(); // MADARA

    let account = SingleOwnerAccount::new(
        provider,
        signer,
        account_address,
        chain_id,
        ExecutionEncoding::New,
    );

    println!("🔢 Calling CounterWithEvent increment()...");
    println!("From account: {:#064x}", account_address);
    println!("Contract: 0x054c37c00efac0bff8a90f83707678116caae53b91d77e1ad287ee95ca2433fa\n");

    let contract_address = Felt::from_hex("0x054c37c00efac0bff8a90f83707678116caae53b91d77e1ad287ee95ca2433fa").unwrap();
    let selector = Felt::from_hex("0x07a44dde9fea32737a5cf3f9683b3235138654aa2d189f6fe44af37a61dc60d").unwrap(); // increment (function_idx 1)

    let call = Call {
        to: contract_address,
        selector,
        calldata: vec![],
    };

    println!("📝 Executing transaction...");
    let result = account.execute_v3(vec![call])
        .l1_gas(50000000)
        .l2_gas(50000000)
        .l1_data_gas(50000)
        .send()
        .await?;

    println!("Transaction hash: {:#064x}\n", result.transaction_hash);
    println!("⏳ Waiting for transaction confirmation...");

    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    let receipt = account.provider().get_transaction_receipt(result.transaction_hash).await?;
    println!("✅ Transaction confirmed!\n");
    println!("Receipt: {:?}", receipt);

    Ok(())
}
