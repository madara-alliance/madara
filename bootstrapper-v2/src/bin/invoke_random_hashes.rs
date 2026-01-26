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

    println!("🔢 Calling Random100Hashes compute_100_hashes()...");
    println!("From account: {:#064x}", account_address);
    println!("Contract: 0x0126078b110d0953b57c3f47e5609f2e81afff7bf24996641a765d1add839174\n");

    let contract_address = Felt::from_hex("0x0126078b110d0953b57c3f47e5609f2e81afff7bf24996641a765d1add839174").unwrap();
    let selector = Felt::from_hex("0x10da2021c3b5aa533b9a5283c5f689031b8081634ba9a7904d74b66feed9d3b").unwrap(); // compute_100_hashes

    let call = Call {
        to: contract_address,
        selector,
        calldata: vec![Felt::from(42)], // seed parameter
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

    let receipt = account.provider().get_transaction_receipt(result.transaction_hash).await?;
    println!("✅ Transaction confirmed!\n");
    println!("Receipt: {:?}", receipt);

    Ok(())
}
