use starknet::{
    accounts::{Account, ConnectedAccount, ExecutionEncoding, SingleOwnerAccount},
    core::types::{contract::{SierraClass, CompiledClass}, Felt},
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider, Url},
    signers::{LocalWallet, SigningKey},
};
use std::{fs, sync::Arc};

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

    println!("🚀 Deploying HeavyTradeSimulator contract...");
    println!("From account: {:#064x}\n", account_address);

    // Read sierra and casm files
    let sierra_path = "/Users/heemankverma/Work/Karnot/RvsC/Kamehameha/compiled/HeavyTradeSimulator.sierra.json";
    let casm_path = "/Users/heemankverma/Work/Karnot/RvsC/Kamehameha/compiled/HeavyTradeSimulator.casm.json";

    let sierra_json = fs::read_to_string(sierra_path)?;
    let casm_json = fs::read_to_string(casm_path)?;

    let sierra_class: SierraClass = serde_json::from_str(&sierra_json)?;
    let casm_class: CompiledClass = serde_json::from_str(&casm_json)?;
    let casm_class_hash = casm_class.class_hash()?;

    println!("📝 Declaring contract class...");
    let flattened = sierra_class.flatten()?;
    let declare_result = account
        .declare_v3(Arc::new(flattened), casm_class_hash)
        .l1_gas(100000000)
        .l2_gas(1000000000)  // Much larger contract needs more gas
        .l1_data_gas(100000)
        .send()
        .await?;

    println!("Declaration hash: {:#064x}", declare_result.transaction_hash);
    println!("Class hash: {:#064x}\n", declare_result.class_hash);

    println!("⏳ Waiting for declaration confirmation...");
    let _receipt = account.provider().get_transaction_receipt(declare_result.transaction_hash).await?;
    println!("✅ Declaration confirmed!\n");

    // Deploy the contract with UDC
    println!("📝 Deploying contract instance via UDC...");

    // Use Universal Deployer Contract for deployment
    let udc_address = Felt::from_hex("0x050b90f8e476202b41e5d151b3691127b39acfc2a43ddcd6460bf725a2a1c659").unwrap();
    let deploy_selector = Felt::from_hex("0x026813d396fdb198e9ead934e4f7a592a8b88a059a09e5e8e7124ab19aa1c2df").unwrap(); // deployContract

    let call = starknet::core::types::Call {
        to: udc_address,
        selector: deploy_selector,
        calldata: vec![
            declare_result.class_hash, // class_hash
            Felt::ZERO,                 // salt
            Felt::ZERO,                 // unique (false)
            Felt::ZERO,                 // constructor calldata length
        ],
    };

    let deploy_tx = account.execute_v3(vec![call])
        .l1_gas(100000000)
        .l2_gas(1000000000)
        .l1_data_gas(100000)
        .send()
        .await?;

    println!("Deploy transaction hash: {:#064x}", deploy_tx.transaction_hash);
    println!("⏳ Waiting for deployment confirmation...");

    let deploy_receipt = account.provider().get_transaction_receipt(deploy_tx.transaction_hash).await?;
    println!("✅ Deployment confirmed!\n");
    println!("Receipt: {:?}\n", deploy_receipt);

    println!("🎉 HeavyTradeSimulator deployed!");
    println!("Class hash: {:#064x}", declare_result.class_hash);
    println!("Transaction: {:#064x}", deploy_tx.transaction_hash);

    Ok(())
}
