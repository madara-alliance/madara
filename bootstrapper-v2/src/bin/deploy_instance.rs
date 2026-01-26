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

    println!("📝 Deploying HeavyTradeSimulator instance via UDC...");

    let udc_address = Felt::from_hex("0x050b90f8e476202b41e5d151b3691127b39acfc2a43ddcd6460bf725a2a1c659").unwrap();
    let deploy_selector = Felt::from_hex("0x037e95a9a40168cca91030e6edbff0cd63b34c9de9ac2e376e5f4f1387f36321").unwrap(); // deployContract
    let class_hash = Felt::from_hex("0x06d89b9177213ba5319b53bbc7b119bbde6ab8184730ef0cd12b4b55315c238f").unwrap();

    let call = Call {
        to: udc_address,
        selector: deploy_selector,
        calldata: vec![
            class_hash,  // class_hash
            Felt::ZERO,  // salt
            Felt::ZERO,  // not_from_zero (false - deploy from zero address)
            Felt::ZERO,  // calldata.len (0 - empty constructor calldata)
        ],
    };

    let deploy_tx = account.execute_v3(vec![call])
        .l1_gas(100000000)
        .l2_gas(100000000)
        .l1_data_gas(100000)
        .send()
        .await?;

    println!("Deploy transaction hash: {:#064x}", deploy_tx.transaction_hash);
    println!("⏳ Waiting for deployment...");

    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

    let receipt = account.provider().get_transaction_receipt(deploy_tx.transaction_hash).await?;
    println!("✅ Deployment confirmed!\n");
    println!("Receipt: {:?}", receipt);

    Ok(())
}
