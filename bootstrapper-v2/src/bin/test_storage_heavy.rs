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
    let chain_id = Felt::from_hex("0x4d4144415241").unwrap();

    let account = SingleOwnerAccount::new(
        provider,
        signer,
        account_address,
        chain_id,
        ExecutionEncoding::New,
    );

    let contract_address = Felt::from_hex("0x0224acb594c268c1f68258f921badf4286b078d674ef51e030f2ad1458ad8c98").unwrap();

    // massive_storage_write selector
    let selector = Felt::from_hex("0x02c6ba2c2f44e63f28697aae1a53321f9579acdd73ba66404223f8b5a8de8058").unwrap();

    println!("🗄️  Testing StorageHeavy contract...");
    println!("Contract: {:#064x}\n", contract_address);

    // Call massive_storage_write with base_value=1000, iteration_count=100 (unused param)
    let base_value = Felt::from(1000u128);
    let iteration_count = Felt::from(100u32);

    let call = Call {
        to: contract_address,
        selector,
        calldata: vec![base_value, iteration_count],
    };

    println!("📝 Executing massive_storage_write...");
    println!("Base value: 1000");
    println!("Iteration count: 100");

    let result = account.execute_v3(vec![call])
        .l1_gas(150000000)
        .l2_gas(150000000)
        .l1_data_gas(150000)
        .send()
        .await?;

    println!("\nTransaction hash: {:#064x}", result.transaction_hash);
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    let receipt = account.provider().get_transaction_receipt(result.transaction_hash).await?;
    println!("\n✅ Storage operation complete!");
    println!("Execution result: {:?}", receipt.receipt.execution_result());
    println!("Events count: {}", receipt.receipt.events().len());

    Ok(())
}
